import { useAtomValue } from "@effect/atom-react";
import {
  AuthTerminalOperateScope,
  type EnvironmentId,
  type ServerProvider,
} from "@t3tools/contracts";
import { useGT } from "gt-react";
import { useState } from "react";
import { resolveOnboardingProviderLoginCommand } from "../../onboarding/providerReadiness.logic";
import { clientHostEnvironmentAtom } from "../../state/clientHostEnvironment";
import { serverEnvironment } from "../../state/server";
import { useEnvironmentScope } from "../../state/session";
import { useAtomCommand } from "../../state/use-atom-command";
import { AgentInstallTerminal } from "../onboarding/AgentInstallTerminal";
import { Button } from "../ui/button";
import { SettingsGroup } from "./SettingsGroup";
import { SettingsRow } from "./settingsLayout";

export function NativeCodexSignIn({
  environmentId,
  provider,
}: {
  readonly environmentId: EnvironmentId;
  readonly provider: ServerProvider;
}) {
  const gt = useGT();
  const [showTerminal, setShowTerminal] = useState(false);
  const config = useAtomValue(serverEnvironment.configValueAtom(environmentId));
  const isClientHost = useAtomValue(clientHostEnvironmentAtom(environmentId));
  const canOperateTerminal = useEnvironmentScope(environmentId, AuthTerminalOperateScope);
  const refresh = useAtomCommand(serverEnvironment.refreshProviders, { reportFailure: false });
  if (!config) return null;

  return (
    <SettingsGroup divided={false}>
      <SettingsRow
        title={gt("Codex account")}
        description={
          isClientHost
            ? undefined
            : gt("Open the link shown in the terminal and enter the device code to sign in.")
        }
        control={
          <Button
            variant="outline"
            disabled={!canOperateTerminal}
            aria-expanded={canOperateTerminal && showTerminal}
            onClick={() => setShowTerminal(true)}
          >
            {isClientHost ? gt("Sign in with ChatGPT") : gt("Sign in with device code")}
          </Button>
        }
      />
      {showTerminal && canOperateTerminal ? (
        <div className="px-4 pb-4" ref={(node) => node?.scrollIntoView({ block: "nearest" })}>
          <AgentInstallTerminal
            session={{
              environmentId,
              driver: provider.driver,
              providerInstanceId: provider.instanceId,
              command: resolveOnboardingProviderLoginCommand(
                provider,
                config.settings,
                config.environment.platform.os,
                { codexLoginMode: isClientHost ? "browser" : "device" },
              ),
              cwd: config.cwd,
              keybindings: config.keybindings,
            }}
            onCommandFinished={() =>
              void refresh({ environmentId, input: { instanceId: provider.instanceId } })
            }
            onClose={() => setShowTerminal(false)}
          />
        </div>
      ) : null}
    </SettingsGroup>
  );
}
