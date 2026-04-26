import {
  forwardRef,
  type ComponentPropsWithoutRef,
  type ReactNode,
} from "react";
import * as RadixTooltip from "@radix-ui/react-tooltip";

import { cn } from "./cn";

export const TooltipProvider = RadixTooltip.Provider;

export interface TooltipProps {
  children: ReactNode;
  content: ReactNode;
  side?: RadixTooltip.TooltipContentProps["side"];
  delayDuration?: number;
  defaultOpen?: boolean;
}

export function Tooltip({
  children,
  content,
  side = "top",
  delayDuration = 200,
  defaultOpen = false,
}: TooltipProps): ReactNode {
  return (
    <RadixTooltip.Root delayDuration={delayDuration} defaultOpen={defaultOpen}>
      <RadixTooltip.Trigger asChild>{children}</RadixTooltip.Trigger>
      <RadixTooltip.Portal>
        <TooltipContent side={side}>{content}</TooltipContent>
      </RadixTooltip.Portal>
    </RadixTooltip.Root>
  );
}

export const TooltipContent = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixTooltip.Content>
>(function TooltipContent({ className, sideOffset = 6, ...rest }, ref) {
  return (
    <RadixTooltip.Content
      ref={ref}
      sideOffset={sideOffset}
      data-pellucid="tooltip-content"
      className={cn(
        "z-50 rounded-[var(--pellucid-radius-sm)] bg-[var(--pellucid-surface-raised)] px-2 py-1 text-xs text-[var(--pellucid-fg)] shadow-[var(--pellucid-shadow-panel)]",
        className,
      )}
      {...rest}
    />
  );
});
