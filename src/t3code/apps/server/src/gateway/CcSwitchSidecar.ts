// @effect-diagnostics nodeBuiltinImport:off
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { createReadStream, existsSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import * as HostProcess from "@t3tools/shared/HostProcess";
import { attachWindowsProcessJob } from "@t3tools/shared/windowsProcessJob";
import * as Context from "effect/Context";
import * as Effect from "effect/Effect";
import * as Layer from "effect/Layer";
import * as Schema from "effect/Schema";

import { ServerConfig } from "../config.ts";

const Ready = Schema.Struct({
  event: Schema.Literal("ready"),
  pid: Schema.Number,
  proxyPort: Schema.Number,
  adminPort: Schema.Number,
  proxyBaseUrl: Schema.String,
  adminBaseUrl: Schema.String,
  version: Schema.String,
});
type Ready = typeof Ready.Type;

const decodeReady = Schema.decodeUnknownSync(Ready);
const gatewayProvider = "t3ccsw_gateway";
export const gatewayTokenEnv = "LOCAL_GATEWAY_TOKEN";

export class GatewayError extends Schema.TaggedError<GatewayError>()("GatewayError", {
  message: Schema.String,
}) {}

export class CcSwitchSidecar extends Context.Service<
  CcSwitchSidecar,
  {
    readonly codexBinary: string;
    readonly codexHome: string;
    readonly nativeCodexHome: string;
    readonly t3Home: string;
    readonly gatewayEnvironment: Readonly<Record<string, string>>;
    readonly launchArgs: string;
    readonly modelCatalog: Effect.Effect<{
      readonly models: ReadonlyArray<string>;
      readonly upstreamModel: string | null;
    } | null>;
    readonly admin: (
      path: string,
      method?: string,
      body?: unknown,
    ) => Effect.Effect<unknown, GatewayError>;
  }
>()("t3/gateway/CcSwitchSidecar") {}

const allowedEnvironment = new Set([
  "PATH",
  "SYSTEMROOT",
  "WINDIR",
  "SYSTEMDRIVE",
  "COMSPEC",
  "PATHEXT",
  "USERPROFILE",
  "LOCALAPPDATA",
  "APPDATA",
  "TEMP",
  "TMP",
  "USERNAME",
  "HOMEDRIVE",
  "HOMEPATH",
]);

export const isolatedChildEnvironment = (environment: NodeJS.ProcessEnv): NodeJS.ProcessEnv =>
  Object.fromEntries(
    Object.entries(environment).filter(([key]) => allowedEnvironment.has(key.toUpperCase())),
  );

const codexLaunchArgs = (proxyBaseUrl: string) =>
  Object.entries({
    model_provider: gatewayProvider,
    [`model_providers.${gatewayProvider}.name`]: "Vikea",
    [`model_providers.${gatewayProvider}.base_url`]: `${proxyBaseUrl}/v1`,
    [`model_providers.${gatewayProvider}.env_key`]: gatewayTokenEnv,
    [`model_providers.${gatewayProvider}.wire_api`]: "responses",
    [`model_providers.${gatewayProvider}.requires_openai_auth`]: false,
    cli_auth_credentials_store: "file",
    mcp_oauth_credentials_store: "file",
    "features.plugins": false,
    "analytics.enabled": false,
    "shell_environment_policy.inherit": "core",
    "shell_environment_policy.exclude": [gatewayTokenEnv],
  })
    .map(([key, value]) => `-c '${key}=${JSON.stringify(value)}'`)
    .join(" ");

interface RunningSidecar {
  readonly child: ChildProcessWithoutNullStreams;
  readonly ready: Ready;
  readonly exited: Promise<void>;
}

function waitForReady(child: ChildProcessWithoutNullStreams, signal: AbortSignal): Promise<Ready> {
  return new Promise((accept, reject) => {
    const stdout = createInterface({ input: child.stdout });
    // @effect-diagnostics-next-line globalTimers:off
    const timeout = setTimeout(() => fail(new Error("Gateway startup timed out")), 15_000);
    const abort = () => fail(new Error("Gateway startup interrupted"));
    const exited = () =>
      fail(new Error(`Gateway exited before it was ready (${child.exitCode ?? child.signalCode})`));
    const cleanup = () => {
      clearTimeout(timeout);
      signal.removeEventListener("abort", abort);
      child.off("error", fail);
      child.off("exit", exited);
      stdout.close();
      child.stdout.resume();
    };
    const fail = (error: unknown) => {
      cleanup();
      reject(error);
    };
    child.once("exit", exited);
    child.once("error", fail);
    signal.addEventListener("abort", abort, { once: true });
    if (signal.aborted) {
      abort();
      return;
    }
    stdout.once("line", (line) => {
      try {
        const ready = decodeReady(JSON.parse(line));
        const validPort = (port: number) => Number.isInteger(port) && port > 0 && port <= 65535;
        if (
          ready.pid !== child.pid ||
          ready.proxyPort === ready.adminPort ||
          !validPort(ready.proxyPort) ||
          !validPort(ready.adminPort) ||
          ready.proxyBaseUrl !== `http://127.0.0.1:${ready.proxyPort}` ||
          ready.adminBaseUrl !== `http://127.0.0.1:${ready.adminPort}`
        ) {
          throw new Error("Gateway advertised invalid listen addresses");
        }
        cleanup();
        accept(ready);
      } catch (error) {
        fail(error);
      }
    });
  });
}

function describeError(error: unknown) {
  return new GatewayError({ message: error instanceof Error ? error.message : String(error) });
}

async function verifyRuntimeAssets(resources: string, artifacts: ReadonlyArray<string>) {
  const manifest = Schema.decodeUnknownSync(Schema.Record(Schema.String, Schema.String))(
    JSON.parse(await readFile(join(resources, "runtime-sha256.json"), "utf8")),
  );
  await Promise.all(
    artifacts.map(async (artifact) => {
      const hash = createHash("sha256");
      for await (const chunk of createReadStream(join(resources, artifact))) hash.update(chunk);
      if (hash.digest("hex") !== manifest[artifact]) {
        throw new Error(`Runtime integrity check failed for ${artifact}`);
      }
    }),
  );
}

function workspaceResourceDirectory(): string {
  let directory = dirname(fileURLToPath(import.meta.url));
  while (dirname(directory) !== directory) {
    if (existsSync(join(directory, "src", "build", "stage-runtime.ps1"))) {
      return join(directory, ".local", "resources");
    }
    directory = dirname(directory);
  }
  throw new Error(
    "Cannot locate VikEA runtime resources. Set T3CCSW_RESOURCES to the staged resource directory.",
  );
}

const make = Effect.fn("CcSwitchSidecar.make")(function* () {
  const config = yield* ServerConfig;
  const environment = yield* HostProcess.Environment;
  const electronResources =
    "resourcesPath" in process && typeof process.resourcesPath === "string"
      ? process.resourcesPath
      : undefined;
  const resources =
    environment.T3CCSW_RESOURCES?.trim() || electronResources || workspaceResourceDirectory();
  const sidecarBinary = resolve(resources, "cc-switch", "cc-switch-sidecar.exe");
  const codexBinary = resolve(resources, "codex", "codex.exe");
  const codexHome = join(config.baseDir, "codex-home");
  const nativeCodexHome = join(config.baseDir, "codex-native-home");
  const dataDir = join(config.baseDir, "proxy-data");
  yield* Effect.tryPromise({
    try: async () => {
      await verifyRuntimeAssets(resources, ["codex/codex.exe", "cc-switch/cc-switch-sidecar.exe"]);
      await Promise.all([
        mkdir(codexHome, { recursive: true }),
        mkdir(nativeCodexHome, { recursive: true }),
        mkdir(dataDir, { recursive: true }),
      ]);
      try {
        await writeFile(
          join(nativeCodexHome, "config.toml"),
          'cli_auth_credentials_store = "file"\nmcp_oauth_credentials_store = "file"\n',
          { flag: "wx" },
        );
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      }
      attachWindowsProcessJob();
    },
    catch: describeError,
  });

  const adminToken = randomBytes(32).toString("hex");
  const gatewayEnvironment = {
    [gatewayTokenEnv]: randomBytes(32).toString("hex"),
    CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED: "1",
  };
  const start = (ports?: Pick<Ready, "adminPort" | "proxyPort">) =>
    Effect.acquireRelease(
      Effect.tryPromise({
        try: async (signal) => {
          const child = spawn(
            sidecarBinary,
            [
              "--data-dir",
              dataDir,
              "--bind-host",
              "127.0.0.1",
              "--proxy-port",
              String(ports?.proxyPort ?? 0),
              "--admin-port",
              String(ports?.adminPort ?? 0),
            ],
            {
              cwd: dataDir,
              windowsHide: true,
              env: {
                ...isolatedChildEnvironment(environment),
                SIDECAR_ADMIN_TOKEN: adminToken,
                [gatewayTokenEnv]: gatewayEnvironment[gatewayTokenEnv],
              },
            },
          );
          child.stdin.on("error", () => {});
          let stderr = "";
          child.stderr.setEncoding("utf8").on("data", (chunk: string) => {
            stderr = (stderr + chunk).slice(-2048);
          });
          const exited = new Promise<void>((resolveExit) =>
            child.once("close", () => resolveExit()),
          );
          try {
            return {
              child,
              ready: await waitForReady(child, signal),
              exited,
            } satisfies RunningSidecar;
          } catch (error) {
            child.kill();
            throw new Error(
              `${describeError(error).message}${stderr.trim() ? ` (${stderr.trim()})` : ""}`,
            );
          }
        },
        catch: describeError,
      }),
      ({ child, exited }) =>
        Effect.promise(async () => {
          child.stdin.end();
          // @effect-diagnostics-next-line globalTimersInEffect:off
          const timer = setTimeout(() => child.kill(), 2000);
          try {
            await exited;
          } finally {
            clearTimeout(timer);
          }
        }),
    );

  const initial = yield* start();
  yield* Effect.forkScoped(
    Effect.gen(function* () {
      yield* Effect.promise(() => initial.exited);
      let delaySeconds = 1;
      while (true) {
        yield* Effect.sleep(`${delaySeconds} seconds`);
        const exit = yield* Effect.exit(
          Effect.scoped(
            Effect.gen(function* () {
              const restarted = yield* start(initial.ready);
              delaySeconds = 1;
              yield* Effect.promise(() => restarted.exited);
            }),
          ),
        );
        if (exit._tag === "Failure") delaySeconds = Math.min(delaySeconds * 2, 30);
      }
    }),
  );

  const admin = (path: string, method = "GET", body?: unknown) =>
    Effect.tryPromise({
      try: async (signal) => {
        // @effect-diagnostics-next-line globalFetchInEffect:off
        const response = await fetch(`${initial.ready.adminBaseUrl}/admin/v1/${path}`, {
          method,
          signal: AbortSignal.any([signal, AbortSignal.timeout(30_000)]),
          redirect: "error",
          headers: {
            Authorization: `Bearer ${adminToken}`,
            ...(body === undefined ? {} : { "Content-Type": "application/json" }),
          },
          // @effect-diagnostics-next-line globalJsonInEffect:off
          ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        });
        if (!response.ok) throw new Error(`Gateway request failed (HTTP ${response.status})`);
        return response.status === 204 ? null : response.json();
      },
      catch: describeError,
    });
  const modelCatalog = admin("health").pipe(
    Effect.map(
      Schema.decodeUnknownOption(
        Schema.Struct({
          activeCodexProvider: Schema.NullOr(
            Schema.Struct({
              models: Schema.Array(Schema.String),
              upstreamModel: Schema.NullOr(Schema.String),
            }),
          ),
        }),
      ),
    ),
    Effect.map((health) => (health._tag === "Some" ? health.value.activeCodexProvider : null)),
    Effect.orElseSucceed(() => null),
  );

  return CcSwitchSidecar.of({
    codexBinary,
    codexHome,
    nativeCodexHome,
    t3Home: config.baseDir,
    gatewayEnvironment,
    launchArgs: codexLaunchArgs(initial.ready.proxyBaseUrl),
    modelCatalog,
    admin,
  });
});

export const layer = Layer.effect(CcSwitchSidecar, make());
