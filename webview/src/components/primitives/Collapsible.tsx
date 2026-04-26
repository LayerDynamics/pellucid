import { forwardRef, type ComponentPropsWithoutRef } from "react";
import * as RadixCollapsible from "@radix-ui/react-collapsible";

import { cn } from "./cn";

export const Collapsible = RadixCollapsible.Root;
export const CollapsibleTrigger = forwardRef<
  HTMLButtonElement,
  ComponentPropsWithoutRef<typeof RadixCollapsible.Trigger>
>(function CollapsibleTrigger({ className, ...rest }, ref) {
  return (
    <RadixCollapsible.Trigger
      ref={ref}
      data-pellucid="collapsible-trigger"
      className={cn(
        "inline-flex h-8 items-center gap-2 rounded-[var(--pellucid-radius-sm)] px-2 text-sm text-[var(--pellucid-fg)] hover:bg-[var(--pellucid-surface-raised)]",
        className,
      )}
      {...rest}
    />
  );
});

export const CollapsibleContent = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixCollapsible.Content>
>(function CollapsibleContent({ className, ...rest }, ref) {
  return (
    <RadixCollapsible.Content
      ref={ref}
      data-pellucid="collapsible-content"
      className={cn(
        "overflow-hidden text-sm text-[var(--pellucid-fg)] data-[state=closed]:hidden",
        className,
      )}
      {...rest}
    />
  );
});
