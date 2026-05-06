import { type ReactElement } from "react";

import { cn } from "../../components/primitives/cn";

export interface CpiComponent {
  /** Component label (e.g. "Food", "Energy"). */
  label: string;
  /** YoY % change. */
  yoyPct: number;
  /** Optional weight (0–1) for stacking the contribution bar. */
  weight?: number;
}

export interface CpiBreakdownProps {
  /** Components to render. */
  components: CpiComponent[];
  /** Headline (already-formatted, e.g. "+3.2% YoY"). */
  headline?: string;
  /** Extra Tailwind classes. */
  className?: string;
}

export function CpiBreakdown(props: CpiBreakdownProps): ReactElement {
  const { components, headline, className } = props;
  if (components.length === 0) {
    return (
      <div
        data-component="CpiBreakdown"
        data-state="empty"
        role="status"
        className={cn("text-[11px] text-[var(--pellucid-muted)]", className)}
      >
        No CPI components.
      </div>
    );
  }
  const max = Math.max(
    1,
    components.reduce((acc, c) => Math.max(acc, Math.abs(c.yoyPct)), 0),
  );
  return (
    <div
      data-component="CpiBreakdown"
      data-state="ready"
      data-component-count={components.length}
      className={cn("flex flex-col gap-1", className)}
    >
      {headline ? (
        <span data-field="headline" className="text-[11px] font-mono text-[var(--pellucid-muted)]">
          {headline}
        </span>
      ) : null}
      <ul className="flex flex-col gap-0.5" aria-label={`${components.length} CPI components`}>
        {components.map((c) => {
          const pct = (Math.abs(c.yoyPct) / max) * 100;
          const tone = c.yoyPct >= 0 ? "positive" : "negative";
          return (
            <li
              key={c.label}
              data-component="CpiBreakdownRow"
              data-label={c.label}
              data-tone={tone}
              className="flex items-center gap-2 text-[11px]"
            >
              <span className="w-20 truncate text-[var(--pellucid-fg)]">{c.label}</span>
              <span
                data-field="yoy"
                className={`w-12 text-right font-mono ${
                  c.yoyPct >= 0 ? "text-[var(--pellucid-success)]" : "text-[var(--pellucid-danger)]"
                }`}
              >
                {c.yoyPct >= 0 ? "+" : ""}{c.yoyPct.toFixed(1)}%
              </span>
              <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-[var(--pellucid-border)]">
                <div
                  data-field="bar"
                  className={`h-full ${
                    c.yoyPct >= 0 ? "bg-[var(--pellucid-success)]" : "bg-[var(--pellucid-danger)]"
                  }`}
                  style={{ width: `${pct.toFixed(2)}%` }}
                />
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
