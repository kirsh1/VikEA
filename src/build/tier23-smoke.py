"""Exercise the isolated Windows gateway and native Codex against loopback fixtures."""

import argparse
import json
import os
from pathlib import Path
import queue
import secrets
import sqlite3
import subprocess
import tempfile
import threading
import time
from contextlib import closing, contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener


ROOT = Path(__file__).resolve().parents[2]
MODEL = "gpt-5.4"
OPENER = build_opener(ProxyHandler({}))


def sse(event, payload):
    prefix = f"event: {event}\n" if event else ""
    return f"{prefix}data: {json.dumps(payload)}\n\n"


def fixture_stream(protocol):
    message = f"fixture {protocol}"
    if protocol == "openai_chat":
        def chunk(choices, **extra):
            return sse(None, dict(id="chat-fixture", object="chat.completion.chunk", created=1,
                                  model=MODEL, choices=choices, **extra))

        return (chunk([dict(index=0, delta=dict(role="assistant", content=message), finish_reason=None)])
                + chunk([dict(index=0, delta={}, finish_reason="stop")])
                + chunk([], usage=dict(prompt_tokens=3, completion_tokens=2, total_tokens=5))
                + "data: [DONE]\n\n")
    if protocol == "anthropic":
        events = [
            ("message_start", dict(message=dict(id="msg-fixture", type="message", role="assistant",
                                               content=[], model=MODEL, stop_reason=None,
                                               usage=dict(input_tokens=3, output_tokens=0)))),
            ("content_block_start", dict(index=0, content_block=dict(type="text", text=""))),
            ("content_block_delta", dict(index=0, delta=dict(type="text_delta", text=message))),
            ("content_block_stop", dict(index=0)),
            ("message_delta", dict(delta=dict(stop_reason="end_turn", stop_sequence=None),
                                   usage=dict(output_tokens=2))),
            ("message_stop", {}),
        ]
        return "".join(sse(event, dict(type=event, **body)) for event, body in events)

    item = dict(id="msg-fixture", type="message", role="assistant", status="completed",
                content=[dict(type="output_text", text=message, annotations=[])])
    response = dict(id="resp-fixture", object="response", created_at=1, status="completed",
                    model=MODEL, output=[item], usage=dict(input_tokens=3, output_tokens=2,
                                                           total_tokens=5))
    events = [
        ("response.created", dict(response={**response, "status": "in_progress", "output": []})),
        ("response.output_item.added", dict(output_index=0, item={**item, "content": []})),
        ("response.output_text.delta", dict(output_index=0, content_index=0,
                                           item_id=item["id"], delta=message)),
        ("response.output_item.done", dict(output_index=0, item=item)),
        ("response.completed", dict(response=response)),
    ]
    return "".join(sse(event, dict(type=event, sequence_number=i, **body))
                   for i, (event, body) in enumerate(events))


@contextmanager
def upstream():
    keys = {protocol: secrets.token_hex(16) for protocol in ("openai_chat", "anthropic", "openai_responses")}
    routes = {"/v1/chat/completions": "openai_chat", "/v1/messages": "anthropic",
              "/v1/responses": "openai_responses"}
    requests = queue.Queue()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_GET(self):
            if self.path == "/catalog-redirect/v1/models":
                self.send_response(302)
                self.send_header("Location", "/v1/models")
                self.end_headers()
                return
            if self.path != "/v1/models":
                self.send_error(404)
                return
            key = keys["anthropic"]
            if (self.headers.get("anthropic-version") != "2023-06-01"
                    or not (self.headers.get("x-api-key") == key
                            or self.headers.get("Authorization") == f"Bearer {key}")):
                self.send_error(401)
                return
            body = json.dumps({"data": [{"id": MODEL}]}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            protocol = routes.get(self.path.split("?", 1)[0])
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            if protocol is None:
                self.send_error(404)
                return
            auth = self.headers.get("x-api-key") if protocol == "anthropic" else self.headers.get("Authorization")
            expected = keys[protocol] if protocol == "anthropic" else f"Bearer {keys[protocol]}"
            if auth != expected:
                self.send_error(401)
                return
            requests.put((protocol, body.get("model")))
            stream = fixture_stream(protocol).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(stream)))
            self.end_headers()
            self.wfile.write(stream)

    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            yield f"http://127.0.0.1:{server.server_port}", keys, requests
        finally:
            server.shutdown()
            worker.join()


def request(base, path, token=None, *, method="GET", body=None, origin=None, expect=200):
    headers = {}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if body is not None:
        headers["Content-Type"] = "application/json"
    if origin:
        headers["Origin"] = origin
    try:
        response = OPENER.open(Request(base + path, method=method, headers=headers,
                                       data=json.dumps(body).encode() if body is not None else None), timeout=15)
    except HTTPError as error:
        response = error
    with response:
        data = response.read().decode()
        assert response.status == expect, (method, path, response.status, data)
        assert "Access-Control-Allow-Origin" not in response.headers
        return json.loads(data) if data and "application/json" in response.headers.get("Content-Type", "") else data


class Process:
    def __init__(self, args, cwd, env, stderr):
        self.inbox = queue.Queue()
        with stderr.open("w", encoding="utf-8") as error_log:
            self.child = subprocess.Popen(args, cwd=cwd, env=env, stdin=subprocess.PIPE,
                                          stdout=subprocess.PIPE, stderr=error_log, text=True,
                                          encoding="utf-8", creationflags=subprocess.CREATE_NO_WINDOW)
        self.stderr = stderr
        self.messages = []
        self.request_id = 0
        threading.Thread(target=self.read_stdout, daemon=True).start()

    def read_stdout(self):
        with self.child.stdout as stdout:
            for line in stdout:
                self.inbox.put(line)
        self.inbox.put(None)

    def send(self, method, params=None, request_id=None):
        message = {"method": method}
        if params is not None:
            message["params"] = params
        if request_id is not None:
            message["id"] = request_id
        self.child.stdin.write(json.dumps(message) + "\n")
        self.child.stdin.flush()

    def until(self, predicate, timeout=30):
        deadline = time.monotonic() + timeout
        try:
            while True:
                line = self.inbox.get(timeout=max(0, deadline - time.monotonic()))
                if line is None:
                    break
                message = json.loads(line)
                self.messages.append(message)
                if predicate(message):
                    return message
        except queue.Empty:
            pass
        raise AssertionError(f"Missing process response. See {self.stderr}. Recent messages: {self.messages[-5:]}")

    def rpc(self, method, params):
        self.request_id += 1
        self.send(method, params, self.request_id)
        reply = self.until(lambda message: message.get("id") == self.request_id and "method" not in message)
        assert "error" not in reply, reply
        return reply["result"]

    def __enter__(self):
        return self

    def __exit__(self, exc_type, *_args):
        self.child.stdin.close()
        try:
            code = self.child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.child.kill()
            code = self.child.wait(timeout=10)
        if exc_type is None:
            assert code == 0, (code, self.stderr)


def codex_args(binary, proxy):
    provider = "t3ccsw_gateway"
    settings = {
        "model_provider": provider,
        f"model_providers.{provider}.name": "VikEA",
        f"model_providers.{provider}.base_url": proxy + "/v1",
        f"model_providers.{provider}.env_key": "LOCAL_GATEWAY_TOKEN",
        f"model_providers.{provider}.wire_api": "responses",
        f"model_providers.{provider}.requires_openai_auth": False,
        "cli_auth_credentials_store": "file",
        "mcp_oauth_credentials_store": "file",
        "features.plugins": False,
        "analytics.enabled": False,
    }
    args = [str(binary), "app-server", "--listen", "stdio://"]
    for key, value in settings.items():
        args.extend(["-c", f"{key}={json.dumps(value)}"])
    return args


def run(sidecar, codex, data):
    include = {"PATH", "SYSTEMROOT", "WINDIR", "SYSTEMDRIVE", "COMSPEC", "PATHEXT",
               "USERPROFILE", "LOCALAPPDATA", "APPDATA", "TEMP", "TMP", "USERNAME"}
    env = {key: value for key, value in os.environ.items() if key.upper() in include}
    env["HTTPS_PROXY"] = "http://127.0.0.1:1"
    admin_token, proxy_token = secrets.token_hex(32), secrets.token_hex(32)
    with upstream() as (base_url, keys, calls), Process(
        [str(sidecar), "--data-dir", str(data / "proxy-data")], data,
        {**env, "SIDECAR_ADMIN_TOKEN": admin_token, "LOCAL_GATEWAY_TOKEN": proxy_token},
        data / "sidecar.stderr.log",
    ) as gateway:
        ready = gateway.until(lambda event: event.get("event") == "ready")
        proxy, admin = ready["proxyBaseUrl"], ready["adminBaseUrl"]
        assert ready["adminPort"] != ready["proxyPort"] and ready["proxyPort"] != 15721, ready
        request(admin, "/admin/v1/health", expect=401)
        request(admin, "/admin/v1/health", proxy_token, expect=401)
        request(admin, "/admin/v1/health", admin_token, origin="http://localhost", expect=403)
        request(proxy, "/health", admin_token, expect=401)
        request(proxy, "/health", proxy_token, origin="http://localhost", expect=403)
        request(admin, "/admin/v1/health", admin_token)
        assert proxy.startswith("http://127.0.0.1:") and admin.startswith("http://127.0.0.1:")

        failover = "/admin/v1/failover/codex"
        assert request(admin, failover, admin_token, method="PUT", expect=400, body={
            "autoFailoverEnabled": True, "queueProviderIds": [],
        })
        proxy_config = "/admin/v1/proxy-config/codex"
        config = request(admin, proxy_config, admin_token)["appProxyConfig"]
        assert request(admin, proxy_config, admin_token, method="PUT", expect=400, body={
            "appProxyConfig": {**config, "autoFailoverEnabled": True},
        })
        providers = {}
        for protocol, key in keys.items():
            created = request(admin, "/admin/v1/providers", admin_token, method="POST", expect=201, body={
                "name": protocol, "protocol": protocol, "baseUrl": base_url + ("" if protocol == "anthropic" else "/v1"),
                "apiKey": key, "upstreamModel": MODEL, "models": [MODEL],
                "anthropicAuthMode": "x-api-key",
            })
            assert key not in json.dumps(created), "Provider DTO exposed upstream credentials"
            saved = created.get("provider", created)
            assert saved["hasApiKey"] and saved["maskedApiKey"] == "••••••••"
            providers[protocol] = saved["id"]

        def activate(protocol):
            request(admin, f"/admin/v1/providers/codex/{providers[protocol]}/activate",
                    admin_token, method="POST", body={})

        first_id = providers["openai_chat"]
        route = request(admin, failover, admin_token, method="PUT", body={
            "autoFailoverEnabled": True, "queueProviderIds": [],
        })
        assert route["autoFailoverEnabled"]
        assert [item["providerId"] for item in route["failoverQueue"]] == [first_id]
        assert request(admin, failover, admin_token, method="PUT", expect=400, body={
            "queueProviderIds": ["missing-provider"],
        })
        before_activation = request(admin, "/admin/v1/providers?app=codex", admin_token)
        assert before_activation["currentProviderId"] == first_id
        assert [p["id"] for p in before_activation["providers"] if p["inFailoverQueue"]] == [first_id]
        activate("anthropic")
        stream = request(proxy, "/v1/responses", proxy_token, method="POST", body={
            "model": MODEL, "input": "hello", "stream": True,
        })
        assert "fixture anthropic" in stream
        assert calls.get(timeout=5) == ("anthropic", MODEL)
        tuned = request(admin, proxy_config, admin_token, method="PUT", body={
            "appProxyConfig": {**config, "autoFailoverEnabled": False, "maxRetries": 2},
        })
        assert not tuned["appProxyConfig"]["autoFailoverEnabled"]
        assert tuned["appProxyConfig"]["maxRetries"] == 2
        assert [item["providerId"] for item in tuned["failoverQueue"]] == [providers["anthropic"], first_id]
        models = request(admin, "/admin/v1/models/fetch", admin_token, method="POST", body={
            "providerId": providers["anthropic"],
        })
        assert [item["id"] for item in models["models"]] == [MODEL]
        for suffix, status in [("", 200), ("/catalog-redirect", 502)]:
            request(admin, "/admin/v1/models/fetch", admin_token, method="POST", expect=status, body={
                "baseUrl": base_url + suffix, "apiKey": keys["anthropic"],
                "protocol": "anthropic", "anthropicAuthMode": "bearer",
            })
        print("Failover activation and native Anthropic discovery without redirects OK", flush=True)

        for protocol in providers:
            activate(protocol)
            stream = request(proxy, "/v1/responses", proxy_token, method="POST", body={
                "model": MODEL, "input": "hello", "stream": True,
            })
            assert f"fixture {protocol}" in stream and "response.completed" in stream, stream
            assert calls.get(timeout=5) == (protocol, MODEL)
            print(f"Gateway SSE OK: {protocol}", flush=True)

        database = data / "proxy-data" / "cc-switch.db"
        with sqlite3.connect(f"{database.as_uri()}?mode=ro", uri=True) as connection:
            rows = connection.execute("SELECT settings_config FROM providers WHERE app_type = 'codex'").fetchall()
        assert len(rows) == len(providers) and all(value.startswith("dpapi:v1:") for value, in rows)
        assert all(key.encode() not in file.read_bytes()
                   for file in database.parent.glob("cc-switch.db*") for key in keys.values())
        print("Windows DPAPI storage OK", flush=True)

        if codex is None:
            return
        codex_home = data / "codex-home"
        codex_home.mkdir()
        del env["HTTPS_PROXY"]
        codex_env = {**env, "CODEX_HOME": str(codex_home), "LOCAL_GATEWAY_TOKEN": proxy_token,
                     "CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED": "1"}
        activate("openai_chat")
        with Process(codex_args(codex, proxy), data, codex_env, data / "codex.stderr.log") as runtime:
            runtime.rpc("initialize", {"clientInfo": {"name": "VikEA Smoke", "version": "1"},
                                       "capabilities": {"experimentalApi": True}})
            runtime.send("initialized")
            account = runtime.rpc("account/read", {"refreshToken": False})
            assert account["account"] is None and account["requiresOpenaiAuth"] is False, account
            thread = runtime.rpc("thread/start", {"model": MODEL, "cwd": str(data),
                                                  "sandbox": "read-only", "approvalPolicy": "never"})["thread"]["id"]
            for protocol in ("openai_chat", "anthropic"):
                activate(protocol)
                previous = len(runtime.messages)
                turn = runtime.rpc("turn/start", {"threadId": thread, "input": [
                    {"type": "text", "text": "Reply with one sentence."},
                ]})["turn"]["id"]
                completed = runtime.until(lambda msg: msg.get("method") == "turn/completed"
                                          and msg["params"]["turn"]["id"] == turn)
                assert completed["params"]["turn"]["status"] == "completed", completed
                assert f"fixture {protocol}" in json.dumps(runtime.messages[previous:])
                assert calls.get(timeout=5) == (protocol, MODEL)
                print(f"Codex turn OK: {protocol}", flush=True)


def check_legacy_credentials(sidecar, data):
    with upstream() as (base_url, keys, calls):
        key = keys["openai_responses"]
        header_secret = secrets.token_hex(16)
        proxy_secret = secrets.token_hex(16)
        admin_token, proxy_token = secrets.token_hex(32), secrets.token_hex(32)
        data_dir = data / "legacy-proxy-data"
        args = [str(sidecar), "--data-dir", str(data_dir)]
        env = {**os.environ, "SIDECAR_ADMIN_TOKEN": admin_token, "LOCAL_GATEWAY_TOKEN": proxy_token}
        with Process(args, data, env, data / "legacy-init.stderr.log") as gateway:
            ready = gateway.until(lambda event: event.get("event") == "ready")
        database = data_dir / "cc-switch.db"
        with closing(sqlite3.connect(database)) as connection, connection:
            connection.execute("DELETE FROM settings WHERE key = 'vikea_dpapi_storage_v1'")
            version, = connection.execute("PRAGMA user_version").fetchone()
            connection.execute(f"PRAGMA user_version = {max(1, version - 1)}")
            connection.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta, is_current) "
                "VALUES ('legacy', 'codex', 'Legacy fixture', ?, ?, 1)",
                (json.dumps({"apiKey": key, "base_url": base_url + "/v1", "model": MODEL,
                             "auth": {"OPENAI_API_KEY": key}, "env": {"OPENAI_API_KEY": key},
                             "config": f'model = "{MODEL}"\nmodel_provider = "gateway"\n'
                                       f'[model_providers.gateway]\nbase_url = "{base_url}/v1"\n'
                                       'wire_api = "responses"\n'}),
                 json.dumps({"apiFormat": "responses", "localProxyRequestOverrides": {
                     "headers": {"x-fixture-secret": header_secret}}})),
            )
            connection.execute("INSERT INTO settings (key, value) VALUES ('global_proxy_url', ?)",
                               (f"http://user:{proxy_secret}@127.0.0.1:1",))
            connection.execute("UPDATE proxy_config SET auto_failover_enabled = 1 WHERE app_type = 'codex'")
        assert key.encode() in database.read_bytes(), "Legacy fixture was not stored in plaintext"
        backups_dir = data_dir / "backups"
        backups_dir.mkdir(exist_ok=True)
        historical_backup = backups_dir / "db_backup_20000101_000000.db"
        with closing(sqlite3.connect(database)) as source, closing(sqlite3.connect(historical_backup)) as dest:
            source.backup(dest)
        assert key.encode() in historical_backup.read_bytes()

        for run_number in (1, 2):
            with Process(args, data, env, data / f"legacy-{run_number}.stderr.log") as gateway:
                ready = gateway.until(lambda event: event.get("event") == "ready")
                provider = request(ready["adminBaseUrl"], "/admin/v1/health", admin_token)["activeCodexProvider"]
                assert provider["hasApiKey"] and provider["maskedApiKey"] == "••••••••"
                assert provider["customHeaders"] == {"x-fixture-secret": ""}
                assert provider["inFailoverQueue"], "Startup must repair the old empty failover queue"
                with closing(sqlite3.connect(f"{database.as_uri()}?mode=ro", uri=True)) as connection:
                    settings, meta = connection.execute(
                        "SELECT settings_config, meta FROM providers WHERE id = 'legacy'"
                    ).fetchone()
                    proxy_url, = connection.execute(
                        "SELECT value FROM settings WHERE key = 'global_proxy_url'"
                    ).fetchone()
                header = json.loads(meta)["localProxyRequestOverrides"]["headers"]["x-fixture-secret"]
                assert all(value.startswith("dpapi:v1:") for value in (settings, header, proxy_url))
                assert all(secret.encode() not in file.read_bytes()
                           for file in [*data_dir.glob("cc-switch.db*"), *backups_dir.glob("*.db*")]
                           for secret in (key, header_secret, proxy_secret))
                # The upstream is loopback and always bypasses the configured proxy.
                stream = request(ready["proxyBaseUrl"], "/v1/responses", proxy_token, method="POST", body={
                    "model": MODEL, "input": "hello", "stream": True,
                })
                assert "fixture openai_responses" in stream
                assert calls.get(timeout=5) == ("openai_responses", MODEL)
        print("Legacy DPAPI migration, backup cleanup, and failover restart repair OK", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sidecar", type=Path, default=ROOT / "src/cc-switch/src-tauri/target/release/cc-switch-sidecar.exe")
    parser.add_argument("--codex", type=Path, default=ROOT / "src/codex/codex-rs/target/x86_64-pc-windows-msvc/release/codex.exe")
    parser.add_argument("--gateway-only", action="store_true")
    args = parser.parse_args()
    runs = ROOT / ".local" / "smoke"
    runs.mkdir(parents=True, exist_ok=True)
    run_dir = Path(tempfile.mkdtemp(dir=runs))
    print(f"Smoke run: {run_dir}", flush=True)
    run(args.sidecar.resolve(strict=True), None if args.gateway_only else args.codex.resolve(strict=True), run_dir)
    check_legacy_credentials(args.sidecar.resolve(strict=True), run_dir)
