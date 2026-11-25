import * as Schema from "effect/Schema";
import { ProviderInstanceId } from "./providerInstance.ts";

// Preserve the instance id already stored in gateway conversations.
export const CC_SWITCH_INSTANCE_ID = ProviderInstanceId.make("codex");
export const VIKEA_NATIVE_CODEX_INSTANCE_ID = ProviderInstanceId.make("codex-native");

export const GatewayProtocol = Schema.Literals(["openai_responses", "openai_chat", "anthropic"]);
export type GatewayProtocol = typeof GatewayProtocol.Type;
const GatewayProviderId = Schema.String.check(Schema.isPattern(/^[A-Za-z0-9_-]{1,100}$/));

export const GatewayProvider = Schema.Struct({
  id: Schema.String,
  name: Schema.String,
  protocol: GatewayProtocol,
  baseUrl: Schema.String,
  hasApiKey: Schema.Boolean,
  maskedApiKey: Schema.NullOr(Schema.String),
  upstreamModel: Schema.NullOr(Schema.String),
  models: Schema.Array(Schema.String),
  anthropicAuthMode: Schema.String,
  maxOutputTokens: Schema.NullOr(Schema.Number),
  customUserAgent: Schema.NullOr(Schema.String),
  customHeaders: Schema.Record(Schema.String, Schema.String),
  isCurrent: Schema.Boolean,
  inFailoverQueue: Schema.Boolean,
  isHealthy: Schema.Boolean,
});
export type GatewayProvider = typeof GatewayProvider.Type;

export const GatewayProviderList = Schema.Struct({
  providers: Schema.Array(GatewayProvider),
  autoFailoverEnabled: Schema.Boolean,
});

export const GatewayHealth = Schema.Struct({
  status: Schema.String,
  pid: Schema.Number,
  dataDir: Schema.String,
  proxyBaseUrl: Schema.String,
  adminBaseUrl: Schema.String,
  activeCodexProvider: Schema.NullOr(GatewayProvider),
  codexHome: Schema.String,
  t3Home: Schema.String,
});

const providerFields = {
  name: Schema.optionalKey(Schema.String),
  protocol: Schema.optionalKey(GatewayProtocol),
  baseUrl: Schema.optionalKey(Schema.String),
  apiKey: Schema.optionalKey(Schema.String),
  clearApiKey: Schema.optionalKey(Schema.Boolean),
  upstreamModel: Schema.optionalKey(Schema.String),
  models: Schema.optionalKey(Schema.Array(Schema.String)),
  anthropicAuthMode: Schema.optionalKey(Schema.Literals(["bearer", "x-api-key"])),
  customHeaders: Schema.optionalKey(Schema.Record(Schema.String, Schema.String)),
  customUserAgent: Schema.optionalKey(Schema.String),
  maxOutputTokens: Schema.optionalKey(Schema.Number),
  clearMaxOutputTokens: Schema.optionalKey(Schema.Boolean),
  inFailoverQueue: Schema.optionalKey(Schema.Boolean),
  sortIndex: Schema.optionalKey(Schema.Number),
};

export const GatewayRequest = Schema.Union([
  Schema.Struct({ operation: Schema.Literals(["health", "list", "getProxyConfig", "usage"]) }),
  Schema.Struct({
    operation: Schema.Literal("create"),
    ...providerFields,
    name: Schema.String,
    baseUrl: Schema.String,
    protocol: GatewayProtocol,
  }),
  Schema.Struct({ operation: Schema.Literal("update"), id: GatewayProviderId, ...providerFields }),
  Schema.Struct({ operation: Schema.Literals(["delete", "probe"]), id: GatewayProviderId }),
  Schema.Struct({
    operation: Schema.Literal("activate"),
    id: GatewayProviderId,
    fallbackIds: Schema.optionalKey(Schema.Array(GatewayProviderId)),
  }),
  Schema.Struct({
    operation: Schema.Literal("fetchModels"),
    providerId: Schema.optionalKey(GatewayProviderId),
    ...providerFields,
  }),
  Schema.Struct({ operation: Schema.Literal("putProxyConfig"), config: Schema.Unknown }),
  Schema.Struct({
    operation: Schema.Literal("putFailover"),
    enabled: Schema.Boolean,
    providerIds: Schema.Array(GatewayProviderId),
  }),
  Schema.Struct({
    operation: Schema.Literal("resetBreakers"),
    providerIds: Schema.Array(GatewayProviderId),
  }),
]);
export type GatewayRequest = typeof GatewayRequest.Type;

export class GatewayRpcError extends Schema.TaggedError<GatewayRpcError>()("GatewayRpcError", {
  message: Schema.String,
}) {}
