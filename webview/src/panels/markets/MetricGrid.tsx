import { type ReactElement } from "react";

import { cn } from "../../components/primitives/cn";

/**
 * One tile in a `MetricGrid`. The grid is purely presentational
 * — every tile renders the same shape and styling.
 */
export interface MetricTile {
  /** Stable id used as the React key + data attribute. */
  id: string;
  /** Short label rendered above the value (e.g. "Price"). */
  label: string;
  /** Big-print value (already-formatted string). */
  value: string;
  /** Optional sub-line rendered below the value (e.g. "+1.24%"). */
  subline?: string;
  /** Optional tone — colours the subline + an accent border. */
  tone?: "positive" | "negative" | "neutral";
}

export interface MetricGridProps {
  /** The tiles to render. */
  tiles: MetricTile[];
  /** Number of CSS-grid columns at the largest breakpoint.
   *  Defaults to 4. The component clamps to `[1, 6]`. */
  columns?: number;
  /** Extra Tailwind classes on the wrapper. */
  className?: string;
}

/**
 * Compact metric tile grid. Used by every market panel that
 * surfaces "N quick numbers" as a header chip-row (price, %
 * change, volume, etc.). Pure — the parent decides which tones
 * to apply.
 */
export function MetricGrid(props: MetricGridProps): ReactElement {
  const { tiles, columns = 4, className } = props;
  const cols = Math.min(6, Math.max(1, Math.floor(columns)));
  const colClass = (() => {
    switch (cols) {
      case 1:
        return "grid-cols-1";
      case 2:
        return "grid-cols-1 sm:grid-cols-2";
      case 3:
        return "grid-cols-2 md:grid-cols-3";
      case 4:
        return "grid-cols-2 md:grid-cols-4";
      case 5:
        return "grid-cols-2 md:grid-cols-3 lg:grid-cols-5";
      default:
        return "grid-cols-2 md:grid-cols-3 lg:grid-cols-6";
    }
  })();

  if (tiles.length === 0) {
    return (
      <div
        data-component="MetricGrid"
        data-state="empty"
        role="status"
        className={cn("text-xs text-[var(--pellucid-muted)]", className)}
      >
        No metrics.
      </div>
    );
  }

  return (
    <div
      data-component="MetricGrid"
      data-state="ready"
      data-tile-count={tiles.length}
      className={cn("grid gap-2", colClass, className)}
    >
      {tiles.map((tile) => (
        <article
          key={tile.id}
          data-component="MetricTile"
          data-tile-id={tile.id}
          data-tone={tile.tone ?? "neutral"}
          className={cn(
            "flex flex-col gap-0.5 rounded-md border bg-[var(--pellucid-surface)] p-2",
            tile.tone === "positive"
              ? "border-[var(--pellucid-success)]"
              : tile.tone === "negative"
                ? "border-[var(--pellucid-danger)]"
                : "border-[var(--pellucid-border)]",
          )}
        >
          <span
            data-field="label"
            className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]"
          >
            {tile.label}
          </span>
          <span
            data-field="value"
            className="text-sm font-semibold text-[var(--pellucid-fg)]"
          >
            {tile.value}
          </span>
          {tile.subline ? (
            <span
              data-field="subline"
              className={cn(
                "text-[11px] font-mono",
                tile.tone === "positive"
                  ? "text-[var(--pellucid-success)]"
                  : tile.tone === "negative"
                    ? "text-[var(--pellucid-danger)]"
                    : "text-[var(--pellucid-muted)]",
              )}
            >
              {tile.subline}
            </span>
          ) : null}
        </article>
      ))}
    </div>
  );
}
