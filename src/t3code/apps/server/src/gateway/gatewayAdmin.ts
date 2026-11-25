import { GatewayRpcError, type GatewayRequest } from "@t3tools/contracts/gateway";
import * as Effect from "effect/Effect";

import { CcSwitchSidecar } from "./CcSwitchSidecar.ts";

export const gatewayAdmin = Effect.fn("gateway.admin")(function* (input: GatewayRequest) {
  const service = yield* Effect.serviceOption(CcSwitchSidecar);
  if (service._tag === "None") {
    return yield* new GatewayRpcError({
      message: "The gateway is unavailable on this environment.",
    });
  }
  const { admin, codexHome, t3Home } = service.value;
  const request = (path: string, method?: string, body?: unknown) =>
    admin(path, method, body).pipe(
      Effect.mapError(({ message }) => new GatewayRpcError({ message })),
    );
  const providerPath = (id: string) => `providers/codex/${encodeURIComponent(id)}`;
  switch (input.operation) {
    case "health": {
      const health = yield* request("health");
      if (health === null || typeof health !== "object") {
        return yield* new GatewayRpcError({ message: "The gateway returned invalid health data." });
      }
      return { ...health, codexHome, t3Home };
    }
    case "list":
      return yield* request("providers?app=codex");
    case "create": {
      const { operation: _operation, ...body } = input;
      return yield* request("providers", "POST", { ...body, app: "codex" });
    }
    case "update": {
      const { operation: _operation, id, ...body } = input;
      return yield* request(providerPath(id), "PATCH", body);
    }
    case "delete":
      return yield* request(providerPath(input.id), "DELETE");
    case "activate":
      return yield* request(`${providerPath(input.id)}/activate`, "POST", {
        ...(input.fallbackIds ? { fallbackIds: input.fallbackIds } : {}),
      });
    case "probe":
      return yield* request(`${providerPath(input.id)}/probe`, "POST");
    case "fetchModels": {
      const { operation: _operation, ...body } = input;
      return yield* request("models/fetch", "POST", { ...body, app: "codex" });
    }
    case "getProxyConfig":
      return yield* request("proxy-config/codex");
    case "putProxyConfig":
      return yield* request("proxy-config/codex", "PUT", input.config);
    case "putFailover":
      return yield* request("failover/codex", "PUT", {
        autoFailoverEnabled: input.enabled,
        queueProviderIds: input.providerIds,
      });
    case "resetBreakers": {
      if (input.providerIds.length === 0)
        return yield* request("circuit-breakers/reset", "POST", { app: "codex" });
      return yield* Effect.forEach(input.providerIds, (providerId) =>
        request("circuit-breakers/reset", "POST", { app: "codex", providerId }),
      );
    }
    case "usage":
      return yield* request("usage/summary?limit=40");
  }
});
