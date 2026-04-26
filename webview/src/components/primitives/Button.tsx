import {
  type ButtonHTMLAttributes,
  forwardRef,
} from "react";
import { Slot } from "@radix-ui/react-slot";

import { cn } from "./cn";

export type ButtonVariant = "solid" | "outline" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  asChild?: boolean;
  loading?: boolean;
}

const VARIANT_CLASSES: Record<ButtonVariant, string> = {
  solid:
    "bg-[var(--pellucid-accent)] text-[var(--pellucid-accent-fg)] hover:opacity-90",
  outline:
    "border border-[var(--pellucid-border)] text-[var(--pellucid-fg)] hover:bg-[var(--pellucid-surface-raised)]",
  ghost:
    "text-[var(--pellucid-fg)] hover:bg-[var(--pellucid-surface-raised)]",
  danger:
    "bg-[var(--pellucid-danger)] text-white hover:opacity-90",
};

const SIZE_CLASSES: Record<ButtonSize, string> = {
  sm: "h-7 px-2 text-xs",
  md: "h-9 px-3 text-sm",
  lg: "h-11 px-4 text-base",
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  function Button(
    {
      variant = "solid",
      size = "md",
      asChild = false,
      loading = false,
      disabled,
      className,
      children,
      type,
      ...rest
    },
    ref,
  ) {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        ref={ref}
        type={asChild ? undefined : (type ?? "button")}
        data-variant-button={variant}
        data-size={size}
        data-loading={loading || undefined}
        aria-busy={loading || undefined}
        disabled={disabled ?? loading}
        className={cn(
          "inline-flex items-center justify-center gap-2 rounded-[var(--pellucid-radius-md)] font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed",
          VARIANT_CLASSES[variant],
          SIZE_CLASSES[size],
          className,
        )}
        {...rest}
      >
        {children}
      </Comp>
    );
  },
);
