import { type ReactElement } from "react";

import { cn } from "../../components/primitives/cn";

/** One watchlist row — symbol + price + delta. */
export interface WatchlistEntry {
  /** Ticker symbol. */
  symbol: string;
  /** Most-recent price. */
  price: number;
  /** Pre-computed % change (positive or negative). */
  percentChange: number;
  /** Optional secondary label (e.g. exchange code). */
  exchange?: string;
  /** Optional currency code displayed next to the price. */
  currency?: string;
  /** Optional click-through href. When set the row renders as a
   *  link, otherwise as a plain row. */
  href?: string;
}

export interface WatchlistRowProps {
  /** The entry to render. */
  entry: WatchlistEntry;
  /** Optional click handler. Fires alongside link navigation. */
  onSelect?: (entry: WatchlistEntry) => void;
  /** Number of fractional digits the price is rendered with.
   *  Defaults to 2. */
  priceDigits?: number;
  /** Extra Tailwind classes. */
  className?: string;
}

/**
 * Compact watchlist row — one stable line per ticker. The
 * markets panels render watchlists, indices, and quote
 * snapshots through this shared row so colour + formatting
 * conventions stay consistent across the family.
 */
export function WatchlistRow(props: WatchlistRowProps): ReactElement {
  const { entry, onSelect, priceDigits = 2, className } = props;
  const tone =
    entry.percentChange > 0
      ? "positive"
      : entry.percentChange < 0
        ? "negative"
        : "neutral";

  const Inner = (
    <>
      <span
        data-field="symbol"
        className="font-mono uppercase text-[var(--pellucid-fg)]"
      >
        {entry.symbol}
      </span>
      <div className="flex items-center gap-2">
        <span
          data-field="price"
          className="font-mono text-[var(--pellucid-fg)]"
        >
          {formatPrice(entry.price, priceDigits)}
          {entry.currency ? (
            <span className="ml-1 text-[10px] text-[var(--pellucid-muted)]">
              {entry.currency}
            </span>
          ) : null}
        </span>
        <span
          data-field="percent-change"
          data-tone={tone}
          className={cn(
            "font-mono text-[11px]",
            tone === "positive" && "text-[var(--pellucid-success)]",
            tone === "negative" && "text-[var(--pellucid-danger)]",
            tone === "neutral" && "text-[var(--pellucid-muted)]",
          )}
        >
          {formatPercent(entry.percentChange)}
        </span>
      </div>
    </>
  );

  if (entry.href) {
    return (
      <a
        data-component="WatchlistRow"
        data-symbol={entry.symbol}
        data-tone={tone}
        href={entry.href}
        target="_blank"
        rel="noreferrer"
        onClick={() => onSelect?.(entry)}
        className={cn(
          "flex items-center justify-between gap-2 rounded border border-transparent px-2 py-1 text-xs hover:border-[var(--pellucid-border)] hover:bg-[var(--pellucid-surface-raised)]",
          className,
        )}
      >
        {Inner}
      </a>
    );
  }
  return (
    <button
      type="button"
      data-component="WatchlistRow"
      data-symbol={entry.symbol}
      data-tone={tone}
      onClick={() => onSelect?.(entry)}
      className={cn(
        "flex w-full items-center justify-between gap-2 rounded border border-transparent px-2 py-1 text-left text-xs hover:border-[var(--pellucid-border)] hover:bg-[var(--pellucid-surface-raised)]",
        className,
      )}
    >
      {Inner}
    </button>
  );
}

/** Format a price with fixed-precision + thousands grouping.
 *  Pure, exported for tests. */
export function formatPrice(value: number, digits: number): string {
  if (!Number.isFinite(value)) return "—";
  return value.toLocaleString("en-US", {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  });
}

/** Format a percent value with sign + 2 fractional digits.
 *  Pure, exported for tests. Zero is rendered as `+0.00%` so a
 *  flat row still picks up the alignment of a signed number. */
export function formatPercent(value: number): string {
  if (!Number.isFinite(value)) return "—";
  const sign = value < 0 ? "" : "+";
  return `${sign}${value.toFixed(2)}%`;
}
