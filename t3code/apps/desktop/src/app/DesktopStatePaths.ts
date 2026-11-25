import * as Option from "effect/Option";

export type JoinPath = (first: string, ...segments: string[]) => string;

function normalizeConfiguredBaseDir(t3Home: Option.Option<string>): Option.Option<string> {
  if (Option.isNone(t3Home)) {
    return Option.none();
  }
  const trimmed = t3Home.value.trim();
  return trimmed.length > 0 ? Option.some(trimmed) : Option.none();
}

export function resolveDesktopBaseDir(input: {
  readonly homeDirectory: string;
  readonly localAppDataDirectory?: string;
  readonly joinPath: JoinPath;
  readonly t3Home: Option.Option<string>;
}): string {
  return Option.getOrElse(normalizeConfiguredBaseDir(input.t3Home), () =>
    input.localAppDataDirectory === undefined
      ? input.joinPath(input.homeDirectory, ".t3ccsw", "t3-home")
      : input.joinPath(input.localAppDataDirectory, "T3CCSW", "t3-home"),
  );
}

export function resolveDesktopStateDir(input: {
  readonly baseDir: string;
  readonly isDevelopment: boolean;
  readonly joinPath: JoinPath;
  readonly t3Home: Option.Option<string>;
}): string {
  const useDevSubdir =
    input.isDevelopment && Option.isNone(normalizeConfiguredBaseDir(input.t3Home));
  return input.joinPath(input.baseDir, useDevSubdir ? "dev" : "userdata");
}
