import {
  ProviderDriverKind,
  type ProviderInstanceConfig,
  type ServerSettings,
} from "@t3tools/contracts";
import { CC_SWITCH_INSTANCE_ID, VIKEA_NATIVE_CODEX_INSTANCE_ID } from "@t3tools/contracts/gateway";
import type { Path } from "effect";
import { isolateNativeCodexHostEnv } from "./codexLaunchArgs.ts";

const CLAUDE_HOME_VARIABLES = ["CLAUDE_CONFIG_DIR"];
const PI_HOME_VARIABLES = ["PI_CODING_AGENT_DIR", "PI_CODING_AGENT_SESSION_DIR"];

/** Windows only scans the app-owned Codex homes and explicitly configured transcript sources. */
export function windowsTranscriptInstances(
  settings: Pick<ServerSettings, "providerInstances">,
  path: Path.Path,
  baseDir: string,
): Readonly<Record<string, ProviderInstanceConfig>> {
  const configured = Object.fromEntries(
    Object.entries(settings.providerInstances).filter(([, instance]) => {
      const config = homeFields(instance.config);
      const hasExplicitVariable = (names: ReadonlyArray<string>) =>
        instance.environment?.some(
          (variable) => names.includes(variable.name.toUpperCase()) && isCustomHome(variable.value),
        ) === true;
      switch (instance.driver) {
        case "codex":
          return isCustomHome(config.homePath) || hasExplicitVariable(["CODEX_HOME"]);
        case "claudeAgent":
          return isCustomHome(config.homePath) || hasExplicitVariable(CLAUDE_HOME_VARIABLES);
        case "pi":
          return hasExplicitVariable(PI_HOME_VARIABLES);
        default:
          return true;
      }
    }),
  );
  const native = settings.providerInstances[VIKEA_NATIVE_CODEX_INSTANCE_ID];
  const nativeConfig = homeFields(native?.config);
  return {
    ...configured,
    [CC_SWITCH_INSTANCE_ID]: {
      driver: ProviderDriverKind.make("codex"),
      config: { homePath: path.join(baseDir, "codex-home") },
    },
    [VIKEA_NATIVE_CODEX_INSTANCE_ID]: {
      ...native,
      driver: ProviderDriverKind.make("codex"),
      config: {
        ...nativeConfig,
        homePath: isCustomHome(nativeConfig.homePath)
          ? nativeConfig.homePath
          : path.join(baseDir, "codex-native-home"),
      },
    },
  };
}

export function windowsTranscriptHostEnvironment(
  driver: string,
  environment: NodeJS.ProcessEnv,
): NodeJS.ProcessEnv {
  if (driver === "codex") return isolateNativeCodexHostEnv(environment);
  const hidden =
    driver === "pi" ? PI_HOME_VARIABLES : driver === "claudeAgent" ? CLAUDE_HOME_VARIABLES : [];
  if (hidden.length === 0) return environment;
  return Object.fromEntries(
    Object.entries(environment).filter(([name]) => !hidden.includes(name.toUpperCase())),
  );
}

function homeFields(config: unknown): Record<string, unknown> {
  return typeof config === "object" && config !== null ? (config as Record<string, unknown>) : {};
}

function isCustomHome(home: unknown): home is string {
  return typeof home === "string" && home.trim().length > 0;
}
