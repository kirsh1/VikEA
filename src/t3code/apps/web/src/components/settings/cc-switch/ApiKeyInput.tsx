// Adapted from CC Switch's forms/ApiKeyInput.tsx. MIT notice: /cc-switch-LICENSE.txt.
import { useGT } from "gt-react";
import { Copy, Eye, EyeOff } from "lucide-react";
import { useId, useState } from "react";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { toastManager } from "../../ui/toast";

export function ApiKeyInput({ placeholder }: { readonly placeholder?: string | undefined }) {
  const gt = useGT();
  const id = useId();
  const [value, setValue] = useState("");
  const [showKey, setShowKey] = useState(false);

  const copyKey = async () => {
    try {
      await navigator.clipboard.writeText(value.trim());
      toastManager.add({ type: "success", title: gt("Copied") });
    } catch {
      toastManager.add({ type: "error", title: gt("Could not copy to the clipboard.") });
    }
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <label htmlFor={id} className="text-sm font-medium">
          {gt("API key")}
        </label>
        {value.trim() ? (
          <button
            type="button"
            onClick={() => void copyKey()}
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
          >
            <Copy className="size-3.5" />
            {gt("Copy")}
          </button>
        ) : null}
      </div>
      <div className="relative">
        <Input
          id={id}
          name="key"
          value={value}
          onChange={(event) => setValue(event.currentTarget.value)}
          type={showKey ? "text" : "password"}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          placeholder={placeholder}
          className="h-8 pr-10"
        />
        {value ? (
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="absolute inset-y-0 right-0"
            onClick={() => setShowKey(!showKey)}
            aria-label={showKey ? gt("Hide API key") : gt("Show API key")}
          >
            {showKey ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
          </Button>
        ) : null}
      </div>
    </div>
  );
}
