import {
  useEffect,
  useMemo,
  useState,
  type ReactElement,
} from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadMarketQuotes,
  type ListMarketQuotesOutcome,
  type ListMarketQuotesQuery,
  type MarketQuote,
} from "../../data/loaders/market/list-market-quotes";
import {
  MetricGrid,
  WatchlistRow,
  registerPanel,
  type MetricTile,
  type WatchlistEntry,
} from "./index";

/** Stable id for `usePanelStore` registration. */
export const MARKET_PANEL_ID = "markets/quotes";

/** Tier gate. The handler is anonymous (FAST-tier cache reader),
 *  so the panel ships at tier 0. */
export const REQUIRED_TIER = 0;

/** Default page size — matches the handler's `DEFAULT_LIMIT`. */
export const DEFAULT_LIMIT = 50;

export interface MarketPanelProps {
  /** Optional pre-filter passed to the loader. */
  query?: ListMarketQuotesQuery;
  /** Override the loader (testing). */
  load?: typeof loadMarketQuotes;
  /** Wall-clock ms used as "now" by the snapshot footer.
   *  Defaults to `Date.now()`; tests override for determinism. */
  now?: number;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: ListMarketQuotesOutcome };

/**
 * Markets panel — M3 family 4.2, T4.2.1.
 *
 * Renders the FAST-tier broad-market quote snapshot the
 * `seed_market_quotes` seeder publishes. Header surfaces
 * 4 quick metrics (basket size + median % change + biggest gainer
 * + biggest loser) via `MetricGrid`, and the body lists every
 * quote as a `WatchlistRow`.
 */
export function MarketPanel(props: MarketPanelProps): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(MARKET_PANEL_ID));

  const [view, setView] = useState<ViewState>({ kind: "loading" });

  const effectiveQuery = useMemo<ListMarketQuotesQuery>(() => {
    const q: ListMarketQuotesQuery = {
      limit: props.query?.limit ?? DEFAULT_LIMIT,
    };
    if (props.query?.symbols && props.query.symbols.length > 0) {
      q.symbols = props.query.symbols;
    }
    return q;
  }, [props.query?.limit, props.query?.symbols]);

  useEffect(() => {
    setLayout(MARKET_PANEL_ID, { rowSpan: 2, colSpan: 2 });
  }, [setLayout]);

  useEffect(() => {
    let cancelled = false;
    if (!hasTier(REQUIRED_TIER)) {
      setView({ kind: "locked", minTier: REQUIRED_TIER });
      return () => {
        cancelled = true;
      };
    }
    setView({ kind: "loading" });
    const loader = props.load ?? loadMarketQuotes;
    void loader(effectiveQuery).then((outcome) => {
      if (cancelled) return;
      setView({ kind: "ready", outcome });
    });
    return () => {
      cancelled = true;
    };
  }, [hasTier, effectiveQuery, props.load]);

  if (isHidden) return <></>;

  return (
    <section
      data-panel-id={MARKET_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Market quotes"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Markets</h3>
      </header>
      {renderBody(view, props.now ?? Date.now())}
    </section>
  );
}

function renderBody(view: ViewState, now: number): ReactElement {
  if (view.kind === "loading") {
    return (
      <div
        role="status"
        aria-live="polite"
        className="text-xs text-[var(--pellucid-muted)]"
      >
        Loading market quotes…
      </div>
    );
  }
  if (view.kind === "locked") {
    return (
      <div
        role="alert"
        data-state="locked"
        className="text-xs text-[var(--pellucid-warn)]"
      >
        Locked — requires tier {view.minTier} or higher.
      </div>
    );
  }
  const out = view.outcome;
  if (out.kind === "ready") {
    if (out.response.rows.length === 0) {
      return (
        <div
          data-state="empty"
          role="status"
          className="text-xs text-[var(--pellucid-muted)]"
        >
          No market quotes match the current filters.
        </div>
      );
    }
    return (
      <div data-state="ready" className="flex flex-col gap-3">
        <MetricGrid tiles={summaryTiles(out.response.rows)} columns={4} />
        <ul
          className="flex flex-col gap-1"
          aria-label={`${out.response.rows.length} market quotes`}
        >
          {out.response.rows.map((q) => (
            <li key={q.symbol}>
              <WatchlistRow entry={toWatchlist(q)} />
            </li>
          ))}
        </ul>
        <footer
          data-component="MarketsFooter"
          className="flex items-center justify-between text-[11px] text-[var(--pellucid-muted)]"
        >
          <span>
            Showing {out.response.rows.length} of {out.response.total}
          </span>
          <span data-field="assembled-at">
            Snapshot: {formatAssembledAt(out.response.assembledAtMs, now)}
          </span>
          {out.response.stale ? (
            <span data-field="stale" aria-live="polite">
              Showing cached snapshot.
            </span>
          ) : null}
        </footer>
      </div>
    );
  }
  if (out.code === "bootstrap_upstream_empty") {
    return (
      <div
        role="alert"
        data-state="outage"
        className="text-xs text-[var(--pellucid-warn)]"
      >
        <strong>Market pipeline offline.</strong>
        <br />
        {out.retryAfterSecs ? (
          <>Retry available in {out.retryAfterSecs}s.</>
        ) : (
          <>Retry shortly.</>
        )}
      </div>
    );
  }
  return (
    <div
      role="alert"
      data-state="error"
      data-error-code={out.code}
      className="text-xs text-[var(--pellucid-danger)]"
    >
      <strong>{labelForCode(out.code)}</strong>
      <br />
      {out.message}
    </div>
  );
}

function toWatchlist(q: MarketQuote): WatchlistEntry {
  const entry: WatchlistEntry = {
    symbol: q.symbol,
    price: q.price,
    percentChange: q.percentChange,
  };
  if (q.exchange) entry.exchange = q.exchange;
  if (q.currency) entry.currency = q.currency;
  return entry;
}

/**
 * Build the summary tiles row from the quote list. Pure —
 * exported so unit tests pin the shape (basket size + median %
 * change + biggest gainer + biggest loser).
 */
export function summaryTiles(rows: MarketQuote[]): MetricTile[] {
  if (rows.length === 0) return [];
  const sorted = [...rows].sort((a, b) => a.percentChange - b.percentChange);
  const median = sorted[Math.floor(sorted.length / 2)]?.percentChange ?? 0;
  const gainer = sorted[sorted.length - 1] ?? sorted[0];
  const loser = sorted[0];
  const tiles: MetricTile[] = [
    {
      id: "basket",
      label: "Basket",
      value: String(rows.length),
    },
    {
      id: "median",
      label: "Median %",
      value: formatPct(median),
      tone: median >= 0 ? "positive" : "negative",
    },
  ];
  if (gainer) {
    tiles.push({
      id: "gainer",
      label: "Top gainer",
      value: gainer.symbol,
      subline: formatPct(gainer.percentChange),
      tone: "positive",
    });
  }
  if (loser && loser !== gainer) {
    tiles.push({
      id: "loser",
      label: "Top loser",
      value: loser.symbol,
      subline: formatPct(loser.percentChange),
      tone: "negative",
    });
  }
  return tiles;
}

function formatPct(value: number): string {
  if (!Number.isFinite(value)) return "—";
  const sign = value < 0 ? "" : "+";
  return `${sign}${value.toFixed(2)}%`;
}

function labelForCode(code: string): string {
  switch (code) {
    case "invalid_request":
      return "Invalid request";
    case "cache_failure":
      return "Cache failure";
    case "cache_shape":
      return "Wire-shape mismatch";
    case "entitlement_forbidden":
      return "Premium tier required";
    case "clerk_unauthorized":
      return "Sign-in required";
    case "network":
      return "Network error";
    default:
      return "Error";
  }
}

/**
 * Format a snapshot timestamp as a relative string ("8m ago",
 * "—" when missing). Pure — exported for tests.
 */
export function formatAssembledAt(timestampMs: number, nowMs: number): string {
  if (!Number.isFinite(timestampMs) || timestampMs <= 0) return "—";
  const delta = Math.max(0, nowMs - timestampMs);
  const sec = Math.floor(delta / 1000);
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const d = new Date(timestampMs);
  const hh = String(d.getUTCHours()).padStart(2, "0");
  const mm = String(d.getUTCMinutes()).padStart(2, "0");
  return `${hh}:${mm} UTC`;
}

// Register in the family registry on import.
registerPanel({
  id: MARKET_PANEL_ID,
  title: "Markets",
  blurb: "Broad-market index/ETF quote snapshot.",
  component: MarketPanel as React.ComponentType<unknown>,
  cacheKeys: ["market:stocks-bootstrap:v1"],
  minTier: REQUIRED_TIER,
  variants: "*",
});
