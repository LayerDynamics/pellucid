import { type ReactElement } from "react";

import { cn } from "../../components/primitives/cn";

export type IndicatorTone = "positive" | "negative" | "neutral";

export interface EconIndicatorTileProps {
  /** Stable id used as data attribute. */
  id: string;
  /** Short uppercase code (e.g. "UNRATE"). */
  code: string;
  /** Human label (e.g. "Unemployment"). */
  label: string;
  /** Big-print value (already-formatted string). */
  value: string;
  /** Optional sub-line (delta, percent change, etc.). */
  subline?: string;
  /** Tone — drives accent border + subline color. */
  tone?: IndicatorTone;
  /** Click handler. */
  onSelect?: () => void;
  /** Extra Tailwind classes. */
  className?: string;
}

const TONE_BORDER: Record<IndicatorTone, string> = {
  positive: "border-[var(--pellucid-success)]",
  negative: "border-[var(--pellucid-danger)]",
  neutral: "border-[var(--pellucid-border)]",
};

const TONE_TEXT: Record<IndicatorTone, string> = {
  positive: "text-[var(--pellucid-success)]",
  negative: "text-[var(--pellucid-danger)]",
  neutral: "text-[var(--pellucid-muted)]",
};

export function EconIndicatorTile(props: EconIndicatorTileProps): ReactElement {
  const { id, code, label, value, subline, tone = "neutral", onSelect, className } = props;
  const Tag = onSelect ? "button" : "div";
  return (
    <Tag
      data-component="EconIndicatorTile"
      data-tile-id={id}
      data-tone={tone}
      onClick={onSelect}
      className={cn(
        "flex flex-col gap-0.5 rounded-md border bg-[var(--pellucid-surface)] p-2 text-left",
        TONE_BORDER[tone],
        onSelect ? "cursor-pointer hover:opacity-80" : "",
        className,
      )}
    >
      <div className="flex items-baseline justify-between">
        <span data-field="code" className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">
          {code}
        </span>
        <span data-field="label" className="text-[10px] text-[var(--pellucid-muted)]">
          {label}
        </span>
      </div>
      <span data-field="value" className="text-base font-mono font-semibold text-[var(--pellucid-fg)]">
        {value}
      </span>
      {subline ? (
        <span data-field="subline" className={`text-[11px] font-mono ${TONE_TEXT[tone]}`}>
          {subline}
        </span>
      ) : null}
    </Tag>
  );
}
