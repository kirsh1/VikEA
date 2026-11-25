// CC Switch presets projected from src/config/codexProviderPresets.ts. MIT notice: /cc-switch-LICENSE.txt.
import type { GatewayProtocol } from "@t3tools/contracts/gateway";
import { useGT } from "gt-react";
import { ChevronRight, Search, SlidersHorizontal } from "lucide-react";
import { useState } from "react";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import catalog from "./presets.json";

export interface GatewayPreset {
  readonly name: string;
  readonly baseUrl: string;
  readonly protocol: GatewayProtocol;
  readonly upstreamModel: string;
  readonly models: ReadonlyArray<string>;
}

const presets = (catalog as ReadonlyArray<GatewayPreset>).toSorted((a, b) =>
  a.name.localeCompare(b.name),
);

export function GatewayPresets({
  onSelect,
  onCancel,
}: {
  readonly onSelect: (preset: GatewayPreset | null) => void;
  readonly onCancel: () => void;
}) {
  const gt = useGT();
  const [query, setQuery] = useState("");
  const queryText = query.trim().toLowerCase();
  const visible = presets.filter((preset) =>
    `${preset.name} ${preset.baseUrl} ${preset.upstreamModel}`.toLowerCase().includes(queryText),
  );
  return (
    <section className="space-y-3 rounded-xl border p-4">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold">{gt("Choose a provider preset")}</h3>
        <Button variant="ghost" size="sm" onClick={onCancel}>
          {gt("Cancel")}
        </Button>
      </div>
      <div className="relative">
        <Search
          aria-hidden
          className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
        />
        <Input
          value={query}
          onChange={(event) => setQuery(event.currentTarget.value)}
          aria-label={gt("Search providers")}
          placeholder={gt("Search providers")}
          className="pl-8"
        />
      </div>
      <button
        type="button"
        className="flex w-full items-center gap-3 rounded-lg border px-3 py-2 text-left text-sm hover:bg-muted/60"
        onClick={() => onSelect(null)}
      >
        <SlidersHorizontal className="size-4" aria-hidden />
        <span className="flex-1">{gt("Custom provider")}</span>
        <ChevronRight className="size-4" aria-hidden />
      </button>
      <div className="max-h-64 overflow-y-auto rounded-lg border divide-y divide-border/60">
        {visible.map((preset, index) => (
          <button
            key={`${preset.name}:${index}`}
            type="button"
            className="flex w-full items-center gap-3 px-3 py-2.5 text-left text-sm hover:bg-muted/60"
            onClick={() => onSelect(preset)}
          >
            <span className="min-w-0 flex-1">
              <span className="block truncate font-medium">{preset.name}</span>
              <span className="block truncate text-xs text-muted-foreground">{preset.baseUrl}</span>
            </span>
            <ChevronRight className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          </button>
        ))}
        {visible.length === 0 ? (
          <p className="p-3 text-sm text-muted-foreground">{gt("No providers found")}</p>
        ) : null}
      </div>
    </section>
  );
}
