import { useAtomValue } from "@effect/atom-react";
import type { EnvironmentId } from "@t3tools/contracts";
import {
  CC_SWITCH_INSTANCE_ID,
  GatewayHealth,
  GatewayProviderList,
  type GatewayProtocol,
  type GatewayProvider,
  type GatewayRequest,
} from "@t3tools/contracts/gateway";
import * as Schema from "effect/Schema";
import { useGT } from "gt-react";
import { Plus, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";

import { useStableCallback } from "../../hooks/useStableCallback";
import { serverEnvironment } from "../../state/server";
import { useAtomCommand } from "../../state/use-atom-command";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Textarea } from "../ui/textarea";
import { ApiKeyInput } from "./cc-switch/ApiKeyInput";
import { GatewayPresets, type GatewayPreset } from "./cc-switch/GatewayPresets";
import { GatewayProviderCard, GatewayProviderEmptyState } from "./cc-switch/GatewayProviderCard";

const readList = Schema.decodeUnknownSync(GatewayProviderList);
const readHealth = Schema.decodeUnknownSync(GatewayHealth);
const readModels = Schema.decodeUnknownSync(
  Schema.Struct({
    models: Schema.Array(Schema.Struct({ id: Schema.String })),
  }),
);
const readProbe = Schema.decodeUnknownSync(
  Schema.Struct({
    success: Schema.Boolean,
    responseTimeMs: Schema.optionalKey(Schema.NullOr(Schema.Number)),
  }),
);
const readUsage = Schema.decodeUnknownSync(
  Schema.Struct({
    totals: Schema.Struct({
      loggedRequests: Schema.Number,
      inputTokens: Schema.Number,
      outputTokens: Schema.Number,
    }),
    recentLogs: Schema.Array(
      Schema.Struct({
        requestId: Schema.String,
        model: Schema.String,
        statusCode: Schema.Number,
        latencyMs: Schema.Number,
        inputTokens: Schema.Number,
        outputTokens: Schema.Number,
      }),
    ),
  }),
);
const objectConfig = Schema.decodeUnknownSync(Schema.Record(Schema.String, Schema.Unknown));
const headerValues = Schema.decodeUnknownSync(Schema.Record(Schema.String, Schema.String));

export function GatewaySettingsPanel({
  environmentId,
  presentation = "settings",
}: {
  readonly environmentId: EnvironmentId;
  readonly presentation?: "settings" | "provider-list" | "provider-picker";
}) {
  const gt = useGT();
  const call = useAtomCommand(serverEnvironment.gateway, { reportFailure: false });
  const permission = useAtomValue(serverEnvironment.gateway.permissionAtom(environmentId));
  const codex = useAtomValue(serverEnvironment.providersValueAtom(environmentId))?.find(
    (provider) => provider.instanceId === CC_SWITCH_INSTANCE_ID,
  );
  const [state, setState] = useState<ReturnType<typeof readList>>();
  const [health, setHealth] = useState<ReturnType<typeof readHealth>>();
  const [usage, setUsage] = useState<ReturnType<typeof readUsage>>();
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [editingChoice, setEditing] = useState<GatewayProvider | "presets" | "new" | null>();
  const [preset, setPreset] = useState<GatewayPreset | null>(null);
  const editing =
    editingChoice === undefined &&
    presentation === "provider-picker" &&
    state?.providers.length === 0
      ? "presets"
      : editingChoice;
  const [config, setConfig] = useState<string>();

  const request = async (input: GatewayRequest) => {
    const result = await call({ environmentId, input });
    if (result._tag !== "Success") throw new Error(gt("The gateway request failed."));
    return result.value;
  };
  const refresh = async () => {
    const [providers, diagnostics, usageData] = await Promise.all([
      request({ operation: "list" }),
      request({ operation: "health" }),
      request({ operation: "usage" }),
    ]);
    setState(readList(providers));
    setHealth(readHealth(diagnostics));
    setUsage(readUsage(usageData));
  };
  const run = useStableCallback(async (action: () => Promise<void>) => {
    setBusy(true);
    setNotice("");
    try {
      await action();
    } catch (error) {
      setNotice(error instanceof Error ? error.message : gt("The gateway request failed."));
    } finally {
      setBusy(false);
    }
  });
  const load = useStableCallback(() => run(refresh));
  // oxlint-disable-next-line t3code/no-unreasoned-state-sync -- Settings reflects the selected server's gateway state.
  useEffect(() => {
    void load();
  }, [environmentId, load]);

  const mutate = (input: GatewayRequest) =>
    run(async () => {
      await request(input);
      await refresh();
    });
  const provider = typeof editing === "object" ? editing : undefined;
  const queue =
    state?.providers.filter((item) => item.inFailoverQueue).map((item) => item.id) ?? [];
  const activeProvider = state?.providers.find((item) => item.isCurrent);
  const automaticFailover = state?.autoFailoverEnabled ?? false;
  const firstMovable = automaticFailover && activeProvider?.id === queue[0] ? 1 : 0;
  const canEnableFailover = activeProvider !== undefined || queue.length > 0;
  const moveFallback = (id: string, step: number) => {
    const index = queue.indexOf(id);
    const next = index + step;
    if (index < firstMovable || next < firstMovable || next >= queue.length) return;
    const reordered = [...queue];
    [reordered[index], reordered[next]] = [reordered[next]!, reordered[index]!];
    void mutate({
      operation: "putFailover",
      enabled: automaticFailover,
      providerIds: reordered,
    });
  };

  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-4 p-4 md:p-6">
      <header className="flex flex-wrap items-center justify-between gap-3">
        {presentation !== "provider-picker" ? (
          <h2 className="text-xl font-semibold">CC Switch</h2>
        ) : null}
        <div className="flex gap-2">
          <Button variant="ghost" disabled={busy} onClick={() => void load()}>
            <RefreshCw className="size-4" />
            {gt("Refresh")}
          </Button>
          <Button
            disabled={busy || !health || Boolean(editing)}
            onClick={() => setEditing("presets")}
          >
            <Plus className="size-4" />
            {gt("Add provider")}
          </Button>
        </div>
      </header>
      {notice && (
        <p role="status" className="text-sm text-muted-foreground">
          {notice}
        </p>
      )}
      {permission && !health && busy && (
        <p className="text-sm text-muted-foreground">{gt("Loading gateway settings…")}</p>
      )}
      {editing === "presets" ? (
        <GatewayPresets
          onSelect={(next) => {
            setPreset(next);
            setEditing("new");
          }}
          onCancel={() => setEditing(null)}
        />
      ) : null}
      {editing && editing !== "presets" && (
        <form
          key={provider?.id ?? "new"}
          className="grid gap-4 rounded-xl border p-4"
          onSubmit={(event) => {
            event.preventDefault();
            const data = new FormData(event.currentTarget);
            const value = (key: string) => String(data.get(key) ?? "").trim();
            void run(async () => {
              const body = {
                name: value("name"),
                baseUrl: value("baseUrl"),
                protocol: value("protocol") as GatewayProtocol,
                ...(value("key") ? { apiKey: value("key") } : {}),
                ...(data.has("clearKey") ? { clearApiKey: true } : {}),
                upstreamModel: value("upstreamModel"),
                models: value("models")
                  .split(/\r?\n|,/)
                  .map((model) => model.trim())
                  .filter(Boolean),
                anthropicAuthMode:
                  value("auth") === "x-api-key" ? ("x-api-key" as const) : ("bearer" as const),
                customUserAgent: value("userAgent"),
                ...(value("maxTokens")
                  ? { maxOutputTokens: Number(value("maxTokens")) }
                  : { clearMaxOutputTokens: true }),
                ...(value("headers")
                  ? { customHeaders: headerValues(JSON.parse(value("headers"))) }
                  : {}),
              };
              await request(
                provider
                  ? { operation: "update", id: provider.id, ...body }
                  : { operation: "create", ...body },
              );
              setEditing(null);
              await refresh();
            });
          }}
        >
          <header className="flex items-center justify-between gap-2">
            <h3 className="text-sm font-semibold">
              {provider ? gt("Edit provider") : gt("Add provider")}
            </h3>
            {!provider ? (
              <Button type="button" variant="ghost" size="sm" onClick={() => setEditing("presets")}>
                {gt("Back to presets")}
              </Button>
            ) : null}
          </header>
          <label className="grid gap-1.5 text-sm font-medium">
            {gt("Provider name")}
            <Input name="name" defaultValue={provider?.name ?? preset?.name} required />
          </label>
          <label className="grid gap-1 text-sm">
            {gt("Protocol")}
            <select
              name="protocol"
              defaultValue={provider?.protocol ?? preset?.protocol ?? "openai_responses"}
              className="h-9 rounded-md border bg-background px-2"
            >
              <option value="openai_responses">OpenAI Responses</option>
              <option value="openai_chat">OpenAI Chat Completions</option>
              <option value="anthropic">Anthropic Messages</option>
            </select>
          </label>
          <label className="grid gap-1 text-sm">
            {gt("Base URL")}
            <Input
              type="url"
              name="baseUrl"
              defaultValue={provider?.baseUrl ?? preset?.baseUrl}
              required
            />
          </label>
          <ApiKeyInput placeholder={provider?.maskedApiKey ?? undefined} />
          {provider?.hasApiKey && (
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" name="clearKey" />
              {gt("Clear saved API key")}
            </label>
          )}
          <label className="grid gap-1 text-sm">
            {gt("Upstream model")}
            <Input
              name="upstreamModel"
              defaultValue={provider?.upstreamModel ?? preset?.upstreamModel ?? ""}
            />
          </label>
          <label className="grid gap-1 text-sm">
            {gt("Model IDs")}
            <Textarea
              name="models"
              defaultValue={(provider?.models ?? preset?.models)?.join("\n")}
            />
          </label>
          <details className="rounded-lg border p-3">
            <summary className="cursor-pointer text-sm font-medium">{gt("Advanced")}</summary>
            <div className="mt-3 grid gap-3 sm:grid-cols-2">
              <label className="grid gap-1 text-sm">
                {gt("Anthropic authentication")}
                <select
                  name="auth"
                  defaultValue={provider?.anthropicAuthMode ?? "x-api-key"}
                  className="h-9 rounded-md border bg-background px-2"
                >
                  <option value="x-api-key">x-api-key</option>
                  <option value="bearer">Bearer</option>
                </select>
              </label>
              <label className="grid gap-1 text-sm">
                {gt("Maximum output tokens")}
                <Input
                  name="maxTokens"
                  type="number"
                  min="1"
                  defaultValue={provider?.maxOutputTokens ?? ""}
                />
              </label>
              <label className="grid gap-1 text-sm">
                {gt("Custom user agent")}
                <Input name="userAgent" defaultValue={provider?.customUserAgent ?? ""} />
              </label>
              <label className="grid gap-1 text-sm">
                {gt("Replace custom headers (JSON)")}
                <Textarea
                  name="headers"
                  placeholder={Object.keys(provider?.customHeaders ?? {}).join(", ")}
                />
              </label>
            </div>
          </details>
          <div className="flex justify-end gap-2">
            <Button type="submit" disabled={busy}>
              {gt("Save provider")}
            </Button>
            <Button type="button" variant="ghost" onClick={() => setEditing(null)}>
              {gt("Cancel")}
            </Button>
          </div>
        </form>
      )}
      {!editing && state?.providers.length === 0 ? (
        <GatewayProviderEmptyState onCreate={() => setEditing("presets")} />
      ) : null}
      {editing
        ? null
        : state?.providers.map((item) => (
            <GatewayProviderCard
              key={item.id}
              provider={item}
              busy={busy}
              requiredQueueHead={automaticFailover && item.isCurrent}
              onEdit={() => {
                setPreset(null);
                setEditing(item);
              }}
              onActivate={() => void mutate({ operation: "activate", id: item.id })}
              onTest={() =>
                void run(async () => {
                  const probe = readProbe(await request({ operation: "probe", id: item.id }));
                  setNotice(
                    !probe.success
                      ? gt("Connection test failed.")
                      : probe.responseTimeMs == null
                        ? gt("Connection test passed.")
                        : gt("Connection test passed in {latency} ms.", {
                            latency: probe.responseTimeMs,
                          }),
                  );
                })
              }
              onFetchModels={() =>
                void run(async () => {
                  const models = readModels(
                    await request({ operation: "fetchModels", providerId: item.id }),
                  ).models;
                  await request({
                    operation: "update",
                    id: item.id,
                    models: models.map(({ id }) => id),
                  });
                  await refresh();
                })
              }
              onToggleFallback={() =>
                void mutate({
                  operation: "update",
                  id: item.id,
                  inFailoverQueue: !item.inFailoverQueue,
                })
              }
              onMoveUp={
                queue.indexOf(item.id) > firstMovable ? () => moveFallback(item.id, -1) : undefined
              }
              onMoveDown={
                queue.indexOf(item.id) >= firstMovable && queue.indexOf(item.id) < queue.length - 1
                  ? () => moveFallback(item.id, 1)
                  : undefined
              }
              onDelete={() => {
                if (window.confirm(gt("Delete {name}?", { name: item.name })))
                  void mutate({ operation: "delete", id: item.id });
              }}
            />
          ))}
      {health && (
        <details className="rounded-lg border p-4">
          <summary className="cursor-pointer font-medium">
            {gt("Gateway settings and diagnostics")}
          </summary>
          <div className="mt-3 grid gap-3 text-sm">
            <div
              className="flex flex-wrap gap-x-5 gap-y-1 rounded-lg bg-muted/40 p-3"
              aria-live="polite"
            >
              <span>
                {gt("Control plane: {status}", { status: health ? gt("Online") : gt("Waiting") })}
              </span>
              <span>
                {gt("Gateway: {status}", {
                  status:
                    health?.activeCodexProvider?.name ??
                    (health ? gt("No active provider") : gt("Waiting")),
                })}
              </span>
              <span>
                {gt("Codex: {status}", {
                  status:
                    codex?.installed && codex.version ? gt("Handshake complete") : gt("Waiting"),
                })}
              </span>
            </div>
            <label className="flex items-center gap-2">
              <input
                type="checkbox"
                disabled={busy || (!automaticFailover && !canEnableFailover)}
                checked={automaticFailover}
                onChange={(event) =>
                  void mutate({
                    operation: "putFailover",
                    enabled: event.currentTarget.checked,
                    providerIds: queue,
                  })
                }
              />
              {gt("Automatic failover")}
            </label>
            {automaticFailover ? (
              <p className="text-muted-foreground">
                {gt("The active provider is tried first, followed by the fallback queue.")}
              </p>
            ) : null}
            <div className="flex gap-2">
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    const { failoverQueue: _queue, ...writable } = objectConfig(
                      await request({ operation: "getProxyConfig" }),
                    );
                    setConfig(JSON.stringify(writable, null, 2));
                  })
                }
              >
                {gt("Edit proxy configuration")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => void mutate({ operation: "resetBreakers", providerIds: [] })}
              >
                {gt("Reset circuit breakers")}
              </Button>
            </div>
            {config !== undefined && (
              <>
                <Textarea
                  className="min-h-40 font-mono"
                  aria-label={gt("Proxy configuration JSON")}
                  value={config}
                  onChange={(event) => setConfig(event.currentTarget.value)}
                />
                <Button
                  size="sm"
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      await request({
                        operation: "putProxyConfig",
                        config: objectConfig(JSON.parse(config)),
                      });
                      setConfig(undefined);
                      await refresh();
                    })
                  }
                >
                  {gt("Save proxy configuration")}
                </Button>
              </>
            )}
            <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-2 break-all">
              <dt>T3CCSW_HOME</dt>
              <dd>{health.t3Home}</dd>
              <dt>CODEX_HOME</dt>
              <dd>{health.codexHome}</dd>
              <dt>{gt("Gateway data")}</dt>
              <dd>{health.dataDir}</dd>
              <dt>{gt("Gateway PID")}</dt>
              <dd>{health.pid}</dd>
              <dt>{gt("Proxy address")}</dt>
              <dd>{health.proxyBaseUrl}</dd>
              <dt>{gt("Admin address")}</dt>
              <dd>{health.adminBaseUrl}</dd>
              <dt>{gt("Codex version")}</dt>
              <dd>{codex?.version ?? gt("Unknown")}</dd>
            </dl>
          </div>
        </details>
      )}
      {usage && (
        <section className="grid gap-3">
          <h2 className="font-semibold">{gt("Usage")}</h2>
          <p className="text-sm text-muted-foreground">
            {gt("{requests} requests · {input} input tokens · {output} output tokens", {
              requests: usage.totals.loggedRequests,
              input: usage.totals.inputTokens,
              output: usage.totals.outputTokens,
            })}
          </p>
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead>
                <tr>
                  <th>{gt("Model")}</th>
                  <th>{gt("Status")}</th>
                  <th>{gt("Latency (ms)")}</th>
                  <th>{gt("Input")}</th>
                  <th>{gt("Output")}</th>
                </tr>
              </thead>
              <tbody>
                {usage.recentLogs.map((entry) => (
                  <tr key={entry.requestId} className="border-t">
                    <td className="py-2">{entry.model}</td>
                    <td>{entry.statusCode}</td>
                    <td>{entry.latencyMs}</td>
                    <td>{entry.inputTokens}</td>
                    <td>{entry.outputTokens}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}
    </div>
  );
}
