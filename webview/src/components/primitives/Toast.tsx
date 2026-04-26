import { forwardRef, type ComponentPropsWithoutRef } from "react";
import * as RadixToast from "@radix-ui/react-toast";

import { cn } from "./cn";

export const ToastProvider = RadixToast.Provider;

export const ToastViewport = forwardRef<
  HTMLOListElement,
  ComponentPropsWithoutRef<typeof RadixToast.Viewport>
>(function ToastViewport({ className, ...rest }, ref) {
  return (
    <RadixToast.Viewport
      ref={ref}
      data-pellucid="toast-viewport"
      className={cn(
        "fixed bottom-4 right-4 z-50 flex w-96 max-w-full flex-col gap-2 outline-none",
        className,
      )}
      {...rest}
    />
  );
});

export type ToastTone = "info" | "success" | "warning" | "danger";

export interface ToastProps
  extends ComponentPropsWithoutRef<typeof RadixToast.Root> {
  tone?: ToastTone;
}

const TONE_CLASSES: Record<ToastTone, string> = {
  info: "border-[var(--pellucid-border)]",
  success: "border-[var(--pellucid-success)]",
  warning: "border-[var(--pellucid-accent)]",
  danger: "border-[var(--pellucid-danger)]",
};

export const Toast = forwardRef<HTMLLIElement, ToastProps>(
  function Toast({ className, tone = "info", ...rest }, ref) {
    return (
      <RadixToast.Root
        ref={ref}
        data-pellucid="toast"
        data-tone={tone}
        className={cn(
          "rounded-[var(--pellucid-radius-md)] border bg-[var(--pellucid-surface-raised)] p-3 shadow-[var(--pellucid-shadow-panel)] text-[var(--pellucid-fg)]",
          TONE_CLASSES[tone],
          className,
        )}
        {...rest}
      />
    );
  },
);

export const ToastTitle = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixToast.Title>
>(function ToastTitle({ className, ...rest }, ref) {
  return (
    <RadixToast.Title
      ref={ref}
      className={cn("text-sm font-medium", className)}
      {...rest}
    />
  );
});

export const ToastDescription = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof RadixToast.Description>
>(function ToastDescription({ className, ...rest }, ref) {
  return (
    <RadixToast.Description
      ref={ref}
      className={cn("mt-1 text-xs text-[var(--pellucid-fg-muted)]", className)}
      {...rest}
    />
  );
});

export const ToastAction = RadixToast.Action;
export const ToastClose = RadixToast.Close;
