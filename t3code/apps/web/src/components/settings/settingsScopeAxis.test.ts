import { describe, expect, it } from "vite-plus/test";

import type { SidebarProjectSnapshot } from "../../sidebarProjectGrouping";
import {
  resolveGatewaySettingsTarget,
  selectEnvironmentAxis,
  selectProjectAxis,
  settingsScopeCheckoutChoices,
  settingsScopeCheckoutLabel,
} from "./settingsScopeAxis";

const member = (environmentId: string, physicalProjectKey: string, workspaceRoot = "") =>
  ({
    environmentId,
    physicalProjectKey,
    workspaceRoot,
  }) as SidebarProjectSnapshot["memberProjects"][number];
const group = (...memberProjects: SidebarProjectSnapshot["memberProjects"]) =>
  ({ memberProjects }) as SidebarProjectSnapshot;

describe("settings scope axes", () => {
  it("selects a gateway only on the explicitly chosen, connected and supported machine", () => {
    const connected = { phase: "connected" } as const;
    const gatewayConfig = { environment: { capabilities: { ccSwitchGateway: true } } };
    const first = { environmentId: "first", connection: connected, serverConfig: gatewayConfig };
    const second = { ...first, environmentId: "second" };
    const unsupported = { ...first, environmentId: "unsupported", serverConfig: null };
    const offline = {
      ...first,
      environmentId: "offline",
      connection: { phase: "offline" } as const,
    };
    const machines = [first, second, unsupported, offline];

    expect(resolveGatewaySettingsTarget(machines, undefined)).toEqual({
      eligible: [first, second],
      target: undefined,
    });
    expect(resolveGatewaySettingsTarget(machines, "unsupported").target).toBeUndefined();
    expect(resolveGatewaySettingsTarget(machines, "offline").target).toBeUndefined();
    expect(resolveGatewaySettingsTarget(machines, "second").target).toBe(second);
  });

  it("drops a checkout narrowing from older links when either axis changes", () => {
    const checkout = { project: "app", checkout: "app@first", machine: "first" };
    expect(selectEnvironmentAxis(checkout, "second")).toEqual({
      project: "app",
      machine: "second",
    });
    expect(selectProjectAxis(checkout, "app")).toEqual({ project: "app", machine: "first" });
  });

  it("offers checkouts only where an environment cannot tell them apart", () => {
    const a = member("laptop", "a");
    const b = member("laptop", "b");
    const c = member("server", "c");

    expect(settingsScopeCheckoutChoices(group(a, b, c), { machine: "laptop" })).toEqual([a, b]);
    expect(settingsScopeCheckoutChoices(group(a, b), {})).toEqual([a, b]);
    expect(settingsScopeCheckoutChoices(group(a, b, c), { checkout: "b" })).toEqual([a, b]);
    expect(settingsScopeCheckoutChoices(group(a, b, c), {})).toEqual([]);
    expect(settingsScopeCheckoutChoices(group(a, b, c), { machine: "server" })).toEqual([]);
  });

  it("labels a checkout by its full path only when its folder name repeats", () => {
    const first = member("laptop", "a", "/work/one/app");
    const second = member("laptop", "b", "C:\\work\\two\\app");
    const other = member("laptop", "c", "/work/tools");

    expect(settingsScopeCheckoutLabel(second, [first, second, other])).toBe("C:\\work\\two\\app");
    expect(settingsScopeCheckoutLabel(other, [first, second, other])).toBe("tools");
  });
});
