// Adapted from CC Switch's ProviderCard and ProviderCardActions. MIT notice: /cc-switch-LICENSE.txt.
import type { GatewayProvider } from "@t3tools/contracts/gateway";
import { useGT } from "gt-react";
import { ArrowDown, ArrowUp, Ellipsis, Pencil, Play, Server } from "lucide-react";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import { Menu, MenuItem, MenuPopup, MenuSeparator, MenuTrigger } from "../../ui/menu";

// The gateway operates on its own database, so the upstream empty state only offers Add.
export function GatewayProviderEmptyState({ onCreate }: { readonly onCreate: () => void }) {
  const gt = useGT();
  return (
    <div className="flex flex-col items-center justify-center px-6 py-12 text-center">
      <div className="mb-5 flex size-14 items-center justify-center rounded-2xl border bg-muted/40 text-muted-foreground">
        <Server className="size-6" strokeWidth={1.5} aria-hidden />
      </div>
      <h3 className="text-sm font-semibold">{gt("No providers configured.")}</h3>
      <Button className="mt-5" variant="outline" onClick={onCreate}>
        {gt("Add provider")}
      </Button>
    </div>
  );
}

export function GatewayProviderCard({
  provider,
  busy,
  requiredQueueHead,
  onActivate,
  onEdit,
  onTest,
  onFetchModels,
  onToggleFallback,
  onMoveUp,
  onMoveDown,
  onDelete,
}: {
  readonly provider: GatewayProvider;
  readonly busy: boolean;
  readonly requiredQueueHead: boolean;
  readonly onActivate: () => void;
  readonly onEdit: () => void;
  readonly onTest: () => void;
  readonly onFetchModels: () => void;
  readonly onToggleFallback: () => void;
  readonly onMoveUp?: (() => void) | undefined;
  readonly onMoveDown?: (() => void) | undefined;
  readonly onDelete: () => void;
}) {
  const gt = useGT();
  return (
    <section
      className={cn(
        "group relative rounded-xl border bg-card transition-colors",
        provider.isCurrent && "border-emerald-500/50 bg-emerald-500/5",
      )}
    >
      <div className="flex min-h-16 flex-wrap items-center gap-3 px-3 py-2.5">
        <Server className="size-5 shrink-0 text-muted-foreground" aria-hidden />
        <div className="min-w-24 flex-1">
          <h3 className="truncate text-sm font-semibold" title={provider.name}>
            {provider.name}
          </h3>
          <p className="truncate text-xs text-muted-foreground" title={provider.baseUrl}>
            {provider.baseUrl}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {provider.isCurrent ? (
            <span className="inline-flex items-center gap-1.5 whitespace-nowrap px-2 text-xs font-medium">
              <span
                aria-hidden
                className={cn(
                  "size-1.5 rounded-full",
                  provider.isHealthy ? "bg-emerald-500" : "bg-amber-500",
                )}
              />
              {gt("Active")}
            </span>
          ) : (
            <Button
              size="icon-sm"
              variant="ghost"
              type="button"
              disabled={busy}
              onClick={onActivate}
              aria-label={gt("Activate")}
              title={gt("Activate")}
            >
              <Play className="size-4" strokeWidth={1.5} />
            </Button>
          )}
          {provider.inFailoverQueue && !requiredQueueHead ? (
            <>
              <Button
                size="icon-sm"
                variant="ghost"
                type="button"
                disabled={busy || !onMoveUp}
                onClick={onMoveUp}
                aria-label={gt("Move earlier")}
                title={gt("Move earlier")}
              >
                <ArrowUp className="size-4" strokeWidth={1.5} />
              </Button>
              <Button
                size="icon-sm"
                variant="ghost"
                type="button"
                disabled={busy || !onMoveDown}
                onClick={onMoveDown}
                aria-label={gt("Move later")}
                title={gt("Move later")}
              >
                <ArrowDown className="size-4" strokeWidth={1.5} />
              </Button>
            </>
          ) : null}
          <Button
            size="icon-sm"
            variant="ghost"
            type="button"
            disabled={busy}
            onClick={onEdit}
            aria-label={gt("Edit")}
            title={gt("Edit")}
          >
            <Pencil className="size-4" strokeWidth={1.5} />
          </Button>
          <Menu>
            <MenuTrigger
              render={
                <Button
                  size="icon-sm"
                  variant="ghost"
                  type="button"
                  disabled={busy}
                  aria-label={gt("More actions")}
                  title={gt("More actions")}
                />
              }
            >
              <Ellipsis className="size-4" strokeWidth={1.5} />
            </MenuTrigger>
            <MenuPopup align="end" width="content">
              <MenuItem onClick={onTest}>{gt("Test connection")}</MenuItem>
              <MenuItem onClick={onFetchModels}>{gt("Fetch model list")}</MenuItem>
              {requiredQueueHead ? null : (
                <MenuItem onClick={onToggleFallback}>
                  {provider.inFailoverQueue ? gt("Remove fallback") : gt("Use as fallback")}
                </MenuItem>
              )}
              <MenuSeparator />
              <MenuItem variant="destructive" onClick={onDelete}>
                {gt("Delete")}
              </MenuItem>
            </MenuPopup>
          </Menu>
        </div>
      </div>
    </section>
  );
}
