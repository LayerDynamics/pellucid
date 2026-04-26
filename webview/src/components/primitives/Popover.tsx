import { forwardRef, type ComponentPropsWithoutRef } from "react";
import * as RadixPopover from "@radix-ui/react-popover";

import { cn } from "./cn";

export const Popover = RadixPopover.Root;
export const PopoverTrigger = RadixPopover.Trigger;
export const PopoverAnchor = RadixPopover.Anchor;
export const PopoverClose = RadixPopover.Close;

export const PopoverContent = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixPopover.Content>
>(function PopoverContent({ className, sideOffset = 6, ...rest }, ref) {
  return (
    <RadixPopover.Portal>
      <RadixPopover.Content
        ref={ref}
        sideOffset={sideOffset}
        data-pellucid="popover-content"
        className={cn(
          "z-50 w-72 rounded-[var(--pellucid-radius-md)] border border-[var(--pellucid-border)] bg-[var(--pellucid-surface-raised)] p-3 text-sm text-[var(--pellucid-fg)] shadow-[var(--pellucid-shadow-panel)] focus:outline-none",
          className,
        )}
        {...rest}
      />
    </RadixPopover.Portal>
  );
});
