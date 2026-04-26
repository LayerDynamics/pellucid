import { forwardRef, type ComponentPropsWithoutRef } from "react";
import * as RadixDropdown from "@radix-ui/react-dropdown-menu";

import { cn } from "./cn";

export const DropdownMenu = RadixDropdown.Root;
export const DropdownMenuTrigger = RadixDropdown.Trigger;
export const DropdownMenuPortal = RadixDropdown.Portal;
export const DropdownMenuSub = RadixDropdown.Sub;
export const DropdownMenuRadioGroup = RadixDropdown.RadioGroup;

export const DropdownMenuContent = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixDropdown.Content>
>(function DropdownMenuContent({ className, sideOffset = 4, ...rest }, ref) {
  return (
    <DropdownMenuPortal>
      <RadixDropdown.Content
        ref={ref}
        sideOffset={sideOffset}
        data-pellucid="dropdown-content"
        className={cn(
          "z-50 min-w-[10rem] rounded-[var(--pellucid-radius-md)] border border-[var(--pellucid-border)] bg-[var(--pellucid-surface-raised)] p-1 shadow-[var(--pellucid-shadow-panel)]",
          className,
        )}
        {...rest}
      />
    </DropdownMenuPortal>
  );
});

export const DropdownMenuItem = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixDropdown.Item>
>(function DropdownMenuItem({ className, ...rest }, ref) {
  return (
    <RadixDropdown.Item
      ref={ref}
      data-pellucid="dropdown-item"
      className={cn(
        "flex cursor-pointer select-none items-center gap-2 rounded-[var(--pellucid-radius-sm)] px-2 py-1.5 text-sm text-[var(--pellucid-fg)] outline-none data-[highlighted]:bg-[var(--pellucid-accent)] data-[highlighted]:text-[var(--pellucid-accent-fg)] data-[disabled]:opacity-40 data-[disabled]:pointer-events-none",
        className,
      )}
      {...rest}
    />
  );
});

export const DropdownMenuSeparator = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixDropdown.Separator>
>(function DropdownMenuSeparator({ className, ...rest }, ref) {
  return (
    <RadixDropdown.Separator
      ref={ref}
      className={cn("-mx-1 my-1 h-px bg-[var(--pellucid-border)]", className)}
      {...rest}
    />
  );
});

export const DropdownMenuLabel = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixDropdown.Label>
>(function DropdownMenuLabel({ className, ...rest }, ref) {
  return (
    <RadixDropdown.Label
      ref={ref}
      className={cn(
        "px-2 py-1.5 text-xs uppercase tracking-wide text-[var(--pellucid-fg-muted)]",
        className,
      )}
      {...rest}
    />
  );
});

export const DropdownMenuRadioItem = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixDropdown.RadioItem>
>(function DropdownMenuRadioItem({ className, children, ...rest }, ref) {
  return (
    <RadixDropdown.RadioItem
      ref={ref}
      data-pellucid="dropdown-radio-item"
      className={cn(
        "relative flex cursor-pointer select-none items-center rounded-[var(--pellucid-radius-sm)] px-2 py-1.5 pl-6 text-sm text-[var(--pellucid-fg)] outline-none data-[highlighted]:bg-[var(--pellucid-accent)] data-[highlighted]:text-[var(--pellucid-accent-fg)]",
        className,
      )}
      {...rest}
    >
      <RadixDropdown.ItemIndicator className="absolute left-1.5">
        •
      </RadixDropdown.ItemIndicator>
      {children}
    </RadixDropdown.RadioItem>
  );
});
