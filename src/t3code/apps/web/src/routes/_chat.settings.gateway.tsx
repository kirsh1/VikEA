import { createFileRoute } from "@tanstack/react-router";
import { AuthProvidersManageScope, type EnvironmentId } from "@t3tools/contracts";
import { useGT } from "gt-react";

import { GatewaySettingsPanel } from "../components/settings/GatewaySettingsPanel";
import { useSettingsScope } from "../components/settings/SettingsScopeContext";
import { SettingsScopeNotice } from "../components/settings/SettingsScopeNotice";
import {
  resolveGatewaySettingsTarget,
  settingsScopeEnvironmentLabel,
} from "../components/settings/settingsScopeAxis";
import { Button } from "../components/ui/button";
import { useEnvironments } from "../state/environments";
import { useEnvironmentScope } from "../state/session";

function PermittedGateway({ environmentId }: { environmentId: EnvironmentId }) {
  const gt = useGT();
  const canManage = useEnvironmentScope(environmentId, AuthProvidersManageScope);
  return canManage ? (
    <GatewaySettingsPanel key={environmentId} environmentId={environmentId} />
  ) : (
    <p className="p-8 text-sm text-muted-foreground">
      {gt("Provider management permission is required.")}
    </p>
  );
}

function GatewayRoute() {
  const { search, selectScope } = useSettingsScope();
  const { environments } = useEnvironments();
  const gt = useGT();
  const { eligible, target: environment } = resolveGatewaySettingsTarget(
    environments,
    search.machine,
  );
  if (!environment) {
    return (
      <SettingsScopeNotice
        target="environment"
        eligibleEnvironmentIds={eligible.map((entry) => entry.environmentId)}
      >
        {eligible.length > 0
          ? gt("Choose the device whose CC Switch gateway you want to manage.")
          : gt("Connect a device that supports CC Switch to manage its gateway.")}
      </SettingsScopeNotice>
    );
  }
  return (
    <section id="cc-switch-gateway">
      <div className="mx-auto flex max-w-4xl items-center justify-between gap-2 px-4 pt-4 text-sm text-muted-foreground md:px-6">
        <p>
          {gt("Managing CC Switch on {device}.", {
            device: settingsScopeEnvironmentLabel(environment, environments),
          })}
        </p>
        <Button variant="outline" size="sm" onClick={() => selectScope({})}>
          {gt("Change device")}
        </Button>
      </div>
      <PermittedGateway environmentId={environment.environmentId} />
    </section>
  );
}

export const Route = createFileRoute("/_chat/settings/gateway")({ component: GatewayRoute });
