import { forwardRef, type ComponentPropsWithoutRef } from "react";
import * as RadixTabs from "@radix-ui/react-tabs";

import { cn } from "./cn";

export const Tabs = RadixTabs.Root;

export const TabsList = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixTabs.List>
>(function TabsList({ className, ...rest }, ref) {
  return (
    <RadixTabs.List
      ref={ref}
      data-pellucid="tabs-list"
      className={cn(
        "inline-flex items-center gap-1 rounded-[var(--pellucid-radius-md)] border border-[var(--pellucid-border)] bg-[var(--pellucid-surface-raised)] p-1",
        className,
      )}
      {...rest}
    />
  );
});

export const TabsTrigger = forwardRef<
  HTMLButtonElement,
  ComponentPropsWithoutRef<typeof RadixTabs.Trigger>
>(function TabsTrigger({ className, ...rest }, ref) {
  return (
    <RadixTabs.Trigger
      ref={ref}
      data-pellucid="tabs-trigger"
      className={cn(
        "rounded-[var(--pellucid-radius-sm)] px-3 py-1.5 text-sm text-[var(--pellucid-fg-muted)] transition-colors data-[state=active]:bg-[var(--pellucid-accent)] data-[state=active]:text-[var(--pellucid-accent-fg)] data-[state=active]:font-medium",
        className,
      )}
      {...rest}
    />
  );
});

export const TabsContent = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixTabs.Content>
>(function TabsContent({ className, ...rest }, ref) {
  return (
    <RadixTabs.Content
      ref={ref}
      data-pellucid="tabs-content"
      className={cn("pt-3 focus:outline-none", className)}
      {...rest}
    />
  );
});
