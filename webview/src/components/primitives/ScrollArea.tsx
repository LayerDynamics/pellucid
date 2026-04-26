import { forwardRef, type ComponentPropsWithoutRef, type ReactNode } from "react";
import * as RadixScrollArea from "@radix-ui/react-scroll-area";

import { cn } from "./cn";

export interface ScrollAreaProps
  extends ComponentPropsWithoutRef<typeof RadixScrollArea.Root> {
  orientation?: "vertical" | "horizontal" | "both";
  children?: ReactNode;
}

export const ScrollArea = forwardRef<HTMLDivElement, ScrollAreaProps>(
  function ScrollArea(
    { className, children, orientation = "vertical", ...rest },
    ref,
  ) {
    return (
      <RadixScrollArea.Root
        ref={ref}
        data-pellucid="scroll-root"
        className={cn("relative overflow-hidden", className)}
        {...rest}
      >
        <RadixScrollArea.Viewport
          data-pellucid="scroll-viewport"
          className="h-full w-full rounded-[inherit]"
        >
          {children}
        </RadixScrollArea.Viewport>
        {orientation === "vertical" || orientation === "both" ? (
          <RadixScrollArea.Scrollbar
            orientation="vertical"
            className="flex w-2 touch-none select-none p-0.5 transition-colors"
          >
            <RadixScrollArea.Thumb className="relative flex-1 rounded-full bg-[var(--pellucid-border)]" />
          </RadixScrollArea.Scrollbar>
        ) : null}
        {orientation === "horizontal" || orientation === "both" ? (
          <RadixScrollArea.Scrollbar
            orientation="horizontal"
            className="flex h-2 touch-none select-none p-0.5 transition-colors"
          >
            <RadixScrollArea.Thumb className="relative flex-1 rounded-full bg-[var(--pellucid-border)]" />
          </RadixScrollArea.Scrollbar>
        ) : null}
        <RadixScrollArea.Corner />
      </RadixScrollArea.Root>
    );
  },
);
