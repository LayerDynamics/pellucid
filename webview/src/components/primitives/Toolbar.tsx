import { forwardRef, type ComponentPropsWithoutRef } from "react";
import * as RadixToolbar from "@radix-ui/react-toolbar";

import { cn } from "./cn";

export const Toolbar = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixToolbar.Root>
>(function Toolbar({ className, ...rest }, ref) {
  return (
    <RadixToolbar.Root
      ref={ref}
      data-pellucid="toolbar"
      className={cn(
        "flex w-full items-center gap-2 rounded-[var(--pellucid-radius-md)] border border-[var(--pellucid-border)] bg-[var(--pellucid-surface-raised)] p-1",
        className,
      )}
      {...rest}
    />
  );
});

export const ToolbarButton = forwardRef<
  HTMLButtonElement,
  ComponentPropsWithoutRef<typeof RadixToolbar.Button>
>(function ToolbarButton({ className, ...rest }, ref) {
  return (
    <RadixToolbar.Button
      ref={ref}
      data-pellucid="toolbar-button"
      className={cn(
        "inline-flex h-8 items-center rounded-[var(--pellucid-radius-sm)] px-2 text-sm text-[var(--pellucid-fg)] hover:bg-[var(--pellucid-surface)] focus-visible:bg-[var(--pellucid-surface)]",
        className,
      )}
      {...rest}
    />
  );
});

export const ToolbarSeparator = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixToolbar.Separator>
>(function ToolbarSeparator({ className, ...rest }, ref) {
  return (
    <RadixToolbar.Separator
      ref={ref}
      className={cn("mx-1 h-5 w-px bg-[var(--pellucid-border)]", className)}
      {...rest}
    />
  );
});

export const ToolbarToggleGroup = RadixToolbar.ToggleGroup;
export const ToolbarToggleItem = forwardRef<
  HTMLButtonElement,
  ComponentPropsWithoutRef<typeof RadixToolbar.ToggleItem>
>(function ToolbarToggleItem({ className, ...rest }, ref) {
  return (
    <RadixToolbar.ToggleItem
      ref={ref}
      data-pellucid="toolbar-toggle-item"
      className={cn(
        "inline-flex h-8 items-center rounded-[var(--pellucid-radius-sm)] px-2 text-sm text-[var(--pellucid-fg)] data-[state=on]:bg-[var(--pellucid-accent)] data-[state=on]:text-[var(--pellucid-accent-fg)]",
        className,
      )}
      {...rest}
    />
  );
});

export const ToolbarLink = RadixToolbar.Link;
