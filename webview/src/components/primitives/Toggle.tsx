import { forwardRef, type ComponentPropsWithoutRef } from "react";
import * as RadixToggle from "@radix-ui/react-toggle";

import { cn } from "./cn";

export type ToggleSize = "sm" | "md" | "lg";

const SIZE_CLASSES: Record<ToggleSize, string> = {
  sm: "h-7 px-2 text-xs",
  md: "h-9 px-3 text-sm",
  lg: "h-11 px-4 text-base",
};

export interface ToggleProps
  extends ComponentPropsWithoutRef<typeof RadixToggle.Root> {
  size?: ToggleSize;
}

export const Toggle = forwardRef<HTMLButtonElement, ToggleProps>(
  function Toggle({ className, size = "md", ...rest }, ref) {
    return (
      <RadixToggle.Root
        ref={ref}
        data-pellucid="toggle"
        data-size={size}
        className={cn(
          "inline-flex items-center justify-center rounded-[var(--pellucid-radius-md)] border border-[var(--pellucid-border)] bg-[var(--pellucid-surface-raised)] text-[var(--pellucid-fg)] transition-colors data-[state=on]:bg-[var(--pellucid-accent)] data-[state=on]:text-[var(--pellucid-accent-fg)] data-[state=on]:border-transparent",
          SIZE_CLASSES[size],
          className,
        )}
        {...rest}
      />
    );
  },
);
