import { T } from "gt-react";
import { cn } from "~/lib/utils";

import { CommandBlock } from "./CommandBlock";

/**
 * Setup steps for a computer with the VikEA launcher on its PATH.
 */
export function InstallT3Commands({
  command,
  className,
}: {
  readonly command: string;
  readonly className?: string;
}) {
  return (
    <div className={cn("space-y-2", className)}>
      <p className="text-xs text-muted-foreground">
        <T>Install VikEA and enable its command in Settings → About.</T>
      </p>
      <CommandBlock command={command} />
    </div>
  );
}
