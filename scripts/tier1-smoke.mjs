import { spawn } from "node:child_process";
import { once } from "node:events";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const root = fileURLToPath(new URL("../", import.meta.url));
const runs = join(root, ".local", "smoke");
await mkdir(runs, { recursive: true });
const data = await mkdtemp(join(runs, "t3-"));
const home = join(data, "t3");
const legacyHome = join(data, "official-t3");
const portProbe = createServer().listen(0, "127.0.0.1");
await once(portProbe, "listening");
const { port } = portProbe.address();
portProbe.close();
const baseUrl = `http://127.0.0.1:${port}`;

const packaged = process.argv[2];
const executable = packaged ? join(packaged, "VikEA.exe") : process.execPath;
const resources = packaged ? join(packaged, "resources") : join(root, ".local", "resources");
const entry = packaged ? join(resources, "server.asar", "apps", "server", "dist", "bin.mjs") : "src/bin.ts";

const child = spawn(executable, [entry, "--no-browser"], {
  cwd: join(root, "t3code", "apps", "server"),
  windowsHide: true,
  env: {
    ...process.env,
    T3CCSW_HOME: home,
    T3CODE_HOME: legacyHome,
    // In the packaged case exercise Electron's resource lookup as well as the source override.
    T3CCSW_RESOURCES: packaged ? undefined : resources,
    ...(packaged ? { ELECTRON_RUN_AS_NODE: "1" } : {}),
    T3CCSW_PORT: String(port),
    OPENAI_API_KEY: "unused-fixture-host-key",
    CODEX_HOME: join(data, "official-codex"),
    HTTPS_PROXY: "http://127.0.0.1:1",
  },
  stdio: ["ignore", "pipe", "pipe"],
});
const stopped = once(child, "exit");
let output = "";
child.stdout.setEncoding("utf8").on("data", (chunk) => { output += chunk; });
child.stderr.setEncoding("utf8").on("data", (chunk) => { output += chunk; });
let socket;

async function eventually(action, timeoutMs = 35_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const result = await action();
    if (result) return result;
    await sleep(100);
  }
  throw new Error("Timed out waiting for a smoke-test condition");
}

async function post(path, body, cookie) {
  const response = await fetch(baseUrl + path, {
    method: "POST",
    headers: {
      Origin: baseUrl,
      ...(body ? { "Content-Type": "application/json" } : {}),
      ...(cookie ? { Cookie: cookie } : {}),
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  assert.equal(response.ok, true, `${path} returned HTTP ${response.status}: ${await response.clone().text()}`);
  return response;
}

let rpcId = 0;
function rpc(tag, payload) {
  const id = String(++rpcId);
  return new Promise((resolve, reject) => {
    const cleanup = () => { clearTimeout(timeout); socket.removeEventListener("message", handle); };
    const timeout = setTimeout(() => { cleanup(); reject(new Error(`No response for ${tag}`)); }, 25_000);
    const handle = ({ data }) => {
      const frame = JSON.parse(data);
      for (const message of Array.isArray(frame) ? frame : [frame]) {
        if (String(message.requestId) !== id || message._tag !== "Exit") continue;
        cleanup();
        if (message.exit._tag === "Success") resolve(message.exit.value);
        else reject(new Error(`${tag}: ${JSON.stringify(message.exit)}`));
      }
    };
    socket.addEventListener("message", handle);
    socket.send(JSON.stringify({ _tag: "Request", id, tag, payload, headers: [] }));
  });
}

try {
  const token = await eventually(() => output.match(/pair#token=([a-z0-9-]+)/i)?.[1]);
  const session = await post("/api/auth/browser-session", { credential: token });
  const cookie = session.headers.getSetCookie().map((value) => value.split(";", 1)[0]).join("; ");
  const { ticket } = await (await post("/api/auth/websocket-ticket", undefined, cookie)).json();
  socket = new WebSocket(`ws://127.0.0.1:${port}/ws?orchestrationProtocol=2&wsTicket=${encodeURIComponent(ticket)}`);
  await Promise.race([once(socket, "open"), sleep(10_000).then(() => { throw new Error("WS open timeout"); })]);

  const gateway = (operation, args = {}) => rpc("gateway.manage", { operation, ...args });
  const health = await gateway("health");
  assert.equal(health.t3Home, home);
  assert.equal(health.codexHome, join(home, "codex-home"));
  assert.match(health.proxyBaseUrl, /^http:\/\/127\.0\.0\.1:\d+$/);
  assert.equal(existsSync(legacyHome), false);
  assert.equal(existsSync(join(data, "official-codex")), false);
  const unauthorized = await fetch(health.adminBaseUrl + "/admin/v1/health");
  assert.equal(unauthorized.status, 401);

  const model = "deepseek-flash";
  const { provider } = await gateway("create", {
    name: "Loopback fixture", protocol: "openai_responses", baseUrl: "http://127.0.0.1:1/v1",
    apiKey: "fixture-upstream-secret", upstreamModel: model, models: [model],
  });
  assert.equal(JSON.stringify(provider).includes("fixture-upstream-secret"), false);
  await gateway("activate", { id: provider.id });
  const providers = await gateway("list");
  assert.equal(providers.providers.length, 1);
  assert.equal(providers.providers[0].isCurrent, true);
  const refreshed = await rpc("server.refreshProviders", {});
  console.log("Provider status", refreshed.providers.map(({ driver, status, version, auth, message }) =>
    ({ driver, status, version, auth: auth?.status, message })));
  const ccSwitch = refreshed.providers.find(({ instanceId }) => instanceId === "codex");
  assert.equal(ccSwitch?.driver, "codex");
  assert.equal(ccSwitch.status, "ready");
  const nativeCodex = refreshed.providers.find(({ instanceId }) => instanceId === "codex-native");
  assert.equal(nativeCodex?.driver, "codex", "Native Codex must remain registered alongside CC Switch");
  assert.equal(nativeCodex.enabled, true);
  assert.equal(nativeCodex.models.some(({ slug }) => slug === model), false);
  assert.ok(refreshed.providers.some(({ driver }) => driver === "claudeAgent"),
    "Gateway activation must preserve other local drivers");
  assert.deepEqual(
    ccSwitch.models.map(({ slug, isCustom, isDefault }) => ({ slug, isCustom, isDefault })),
    [{ slug: model, isCustom: false, isDefault: true }],
    "The conversation picker must receive the active CC Switch catalog as provider models",
  );
  const secondModel = "deepseek-v4-pro";
  const alternateModel = "deepseek-v4-flash";
  const { provider: secondProvider } = await gateway("create", {
    name: "Second loopback fixture", protocol: "openai_responses", baseUrl: "http://127.0.0.1:1/v1",
    apiKey: "fixture-upstream-secret", upstreamModel: secondModel, models: [alternateModel, secondModel],
  });
  await gateway("activate", { id: secondProvider.id });
  const switching = await rpc("server.getConfig", {});
  assert.deepEqual(
    switching.providers.find(({ instanceId }) => instanceId === "codex").models
      .map(({ slug, isCustom, isDefault }) => ({ slug, isCustom, isDefault })),
    [
      { slug: alternateModel, isCustom: false, isDefault: false },
      { slug: secondModel, isCustom: false, isDefault: true },
    ],
    "Switching the active provider replaces its catalog without an explicit provider refresh",
  );

  const childIsRunning = (pid) => {
    try { process.kill(pid, 0); return true; }
    catch (error) { if (error.code === "ESRCH") return false; throw error; }
  };
  assert.equal(childIsRunning(health.pid), true);
  socket.close();
  child.kill("SIGKILL");
  await stopped;
  await eventually(() => !childIsRunning(health.pid), 1_000);
  await writeFile(join(data, "job.json"), JSON.stringify({ activeAfterExit: 0, sidecarPid: health.pid }));
  console.log(`Tier 1 gateway RPC and Windows job cleanup passed (${data})`);
} finally {
  socket?.close();
  if (child.exitCode === null && child.signalCode === null) child.kill();
  await stopped;
  await writeFile(join(data, "server.log"), output.replace(/(pair#token=)[a-z0-9-]+/gi, "$1[redacted]"));
}
