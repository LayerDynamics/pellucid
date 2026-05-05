import { type ReactElement } from "react";

import { cn } from "../../components/primitives/cn";

/**
 * Severity tag rendered next to news / intel signals. Shared by
 * `NewsCard`, `BreakingTicker`, `SignalSeverityBadge` (this file
 * — also exported standalone for the GdeltIntelPanel and
 * RegionalIntelligenceBoard signal lists).
 *
 * Severity scale matches the original WorldMonitor badge order:
 *  - `info`     — neutral / FYI; default text color.
 *  - `warn`     — elevated attention; amber.
 *  - `high`     — actionable; orange.
 *  - `critical` — immediate; red.
 *
 * Colors come from the design tokens (`var(--pellucid-*)`) so a
 * variant theme override (T2.8) re-paints the badge without
 * touching this component.
 */

export type SignalSeverity = "info" | "warn" | "high" | "critical";

export interface SignalSeverityBadgeProps {
  /** The severity level. */
  severity: SignalSeverity;
  /** Optional override label. Defaults to a capitalised severity
   *  name (`"Info"`, `"Warn"`, `"High"`, `"Critical"`). */
  label?: string;
  /** When `true`, render a tighter pill (single-letter glyph). */
  compact?: boolean;
  /** Extra Tailwind classes. */
  className?: string;
}

const SEVERITY_CLASS: Record<SignalSeverity, string> = {
  info: "border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] text-[var(--pellucid-muted)]",
  warn: "border-[color:var(--pellucid-warn)] text-[var(--pellucid-warn)]",
  high: "border-[color:var(--pellucid-warn)] bg-[color:var(--pellucid-warn)]/10 text-[var(--pellucid-warn)]",
  critical:
    "border-[color:var(--pellucid-danger)] bg-[color:var(--pellucid-danger)]/10 text-[var(--pellucid-danger)]",
};

const SEVERITY_LABEL: Record<SignalSeverity, string> = {
  info: "Info",
  warn: "Warn",
  high: "High",
  critical: "Critical",
};

const SEVERITY_GLYPH: Record<SignalSeverity, string> = {
  info: "i",
  warn: "!",
  high: "!!",
  critical: "X",
};

export function SignalSeverityBadge(
  props: SignalSeverityBadgeProps,
): ReactElement {
  const { severity, label, compact = false, className } = props;
  const text = label ?? (compact ? SEVERITY_GLYPH[severity] : SEVERITY_LABEL[severity]);
  return (
    <span
      data-component="SignalSeverityBadge"
      data-severity={severity}
      role="img"
      aria-label={`Severity: ${SEVERITY_LABEL[severity]}`}
      className={cn(
        "inline-flex items-center justify-center rounded-full border font-mono uppercase",
        compact ? "h-4 min-w-4 px-1 text-[10px]" : "h-5 min-w-12 px-2 text-[11px]",
        SEVERITY_CLASS[severity],
        className,
      )}
    >
      {text}
    </span>
  );
}

/**
 * Order severities highest → lowest. Useful when sorting a list
 * of signals so the most urgent ones surface first.
 */
export function compareSeverity(
  a: SignalSeverity,
  b: SignalSeverity,
): number {
  const rank: Record<SignalSeverity, number> = {
    critical: 3,
    high: 2,
    warn: 1,
    info: 0,
  };
  return rank[b] - rank[a];
}
