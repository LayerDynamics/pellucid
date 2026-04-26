import { forwardRef, type ComponentPropsWithoutRef, type ReactNode } from "react";
import * as RadixDialog from "@radix-ui/react-dialog";

import { cn } from "./cn";

export const Dialog = RadixDialog.Root;
export const DialogTrigger = RadixDialog.Trigger;
export const DialogClose = RadixDialog.Close;
export const DialogPortal = RadixDialog.Portal;

export const DialogOverlay = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixDialog.Overlay>
>(function DialogOverlay({ className, ...rest }, ref) {
  return (
    <RadixDialog.Overlay
      ref={ref}
      data-pellucid="dialog-overlay"
      className={cn(
        "fixed inset-0 bg-black/60 backdrop-blur-sm",
        className,
      )}
      {...rest}
    />
  );
});

export type DialogContentProps = ComponentPropsWithoutRef<
  typeof RadixDialog.Content
> & {
  heading?: ReactNode;
  blurb?: ReactNode;
};

export const DialogContent = forwardRef<HTMLDivElement, DialogContentProps>(
  function DialogContent(
    { className, children, heading, blurb, "aria-describedby": ariaDescribedBy, ...rest },
    ref,
  ) {
    // Radix warns when neither <Description> nor aria-describedby is given.
    // Explicitly pass `undefined` to opt out when the caller has no blurb.
    const describedBy =
      blurb === undefined && ariaDescribedBy === undefined
        ? undefined
        : ariaDescribedBy;
    return (
      <DialogPortal>
        <DialogOverlay />
        <RadixDialog.Content
          ref={ref}
          data-pellucid="dialog-content"
          aria-describedby={describedBy}
          className={cn(
            "fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 rounded-[var(--pellucid-radius-lg)] border border-[var(--pellucid-border)] bg-[var(--pellucid-surface-raised)] p-6 shadow-[var(--pellucid-shadow-panel)] focus:outline-none",
            className,
          )}
          {...rest}
        >
          {heading !== undefined ? (
            <RadixDialog.Title className="text-base font-semibold text-[var(--pellucid-fg)]">
              {heading}
            </RadixDialog.Title>
          ) : null}
          {blurb !== undefined ? (
            <RadixDialog.Description className="mt-1 text-sm text-[var(--pellucid-fg-muted)]">
              {blurb}
            </RadixDialog.Description>
          ) : null}
          {children}
        </RadixDialog.Content>
      </DialogPortal>
    );
  },
);

export const DialogTitle = RadixDialog.Title;
export const DialogDescription = RadixDialog.Description;
