import {
  useEffect,
  useMemo,
  useState,
  type ReactElement,
} from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadAnalyzeStock,
  type AnalyzeStockOutcome,
  type AnalyzeStockQuery,
  type AnalyzeStockResponse,
} from "../../data/loaders/market/analyze-stock";
import {
  MetricGrid,
  SymbolPicker,
  registerPanel,
  type MetricTile,
} from "./index";

/** Stable id for `usePanelStore` registration. */
export const STOCK_ANALYSIS_PANEL_ID = "markets/stock-analysis";

/** Tier gate. The handler is gated to tier 2 at the gateway —
 *  the panel mirrors that with a client-side check so users
 *  see a "locked" branch instead of a network round-trip when
 *  they don't have the tier. */
export const REQUIRED_TIER = 2;

/** Default symbol shown on first mount. */
export const DEFAULT_SYMBOL = "SPY";

const PRESET_SYMBOLS = ["SPY", "QQQ", "DIA", "IWM", "VTI", "EFA", "EEM", "^VIX"];

export interface StockAnalysisPanelProps {
  /** Initial / pre-set symbol. Defaults to [`DEFAULT_SYMBOL`]. */
  symbol?: string;
  /** Override the loader (testing). */
  load?: typeof loadAnalyzeStock;
  /** Optional bearer token forwarded to the loader. */
  bearerToken?: string;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: AnalyzeStockOutcome };

/**
 * Stock analysis panel — M3 family 4.2, T4.2.2.
 *
 * Tier-2 gated. Renders a per-symbol analytics card built from
 * the `market:stocks-bootstrap:v1` snapshot: dollar change,
 * percent change, trend (up/down/flat), magnitude
 * (small/medium/large), and the symbol's range-position inside
 * the basket. The header has a SymbolPicker bound to the
 * loader's `?symbol=` knob.
 */
export function StockAnalysisPanel(
  props: StockAnalysisPanelProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(STOCK_ANALYSIS_PANEL_ID));

  const [symbol, setSymbol] = useState<string>(
    (props.symbol ?? DEFAULT_SYMBOL).toUpperCase(),
  );
  const [view, setView] = useState<ViewState>({ kind: "loading" });

  const effectiveQuery = useMemo<AnalyzeStockQuery>(
    () => ({ symbol }),
    [symbol],
  );

  useEffect(() => {
    setLayout(STOCK_ANALYSIS_PANEL_ID, { rowSpan: 2, colSpan: 2 });
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
    const loader = props.load ?? loadAnalyzeStock;
    const opts = props.bearerToken
      ? { bearerToken: props.bearerToken }
      : undefined;
    void loader(effectiveQuery, opts).then((outcome) => {
      if (cancelled) return;
      setView({ kind: "ready", outcome });
    });
    return () => {
      cancelled = true;
    };
  }, [hasTier, effectiveQuery, props.load, props.bearerToken]);

  if (isHidden) return <></>;

  return (
    <section
      data-panel-id={STOCK_ANALYSIS_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Stock analysis"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Stock analysis</h3>
        <SymbolPicker
          value={symbol}
          onChange={setSymbol}
          presets={PRESET_SYMBOLS}
        />
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ViewState): ReactElement {
  if (view.kind === "loading") {
    return (
      <div
        role="status"
        aria-live="polite"
        className="text-xs text-[var(--pellucid-muted)]"
      >
        Loading analysis…
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
        Locked — stock analysis requires tier {view.minTier} or higher.
      </div>
    );
  }
  const out = view.outcome;
  if (out.kind === "ready") {
    return (
      <div data-state="ready" className="flex flex-col gap-3">
        <header
          data-component="StockAnalysisHeader"
          className="flex items-baseline justify-between gap-2"
        >
          <h4
            data-field="symbol"
            className="text-base font-semibold uppercase font-mono"
          >
            {out.response.symbol}
          </h4>
          <span
            data-field="exchange"
            className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]"
          >
            {out.response.exchange} · {out.response.currency}
          </span>
        </header>
        <MetricGrid tiles={metricTiles(out.response)} columns={4} />
        <div
          data-component="StockAnalysisRangeBar"
          className="flex flex-col gap-1"
        >
          <div className="flex items-baseline justify-between text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">
            <span>Basket position</span>
            <span data-field="range-position-label">
              {(out.response.metrics.rangePosition * 100).toFixed(0)}%
            </span>
          </div>
          <div
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(
              out.response.metrics.rangePosition * 100,
            )}
            className="relative h-1.5 w-full overflow-hidden rounded-full bg-[var(--pellucid-border)]"
          >
            <div
              data-field="range-position-fill"
              className="absolute left-0 top-0 h-full rounded-full bg-[var(--pellucid-info)]"
              style={{
                width: `${(out.response.metrics.rangePosition * 100).toFixed(2)}%`,
              }}
            />
          </div>
        </div>
        <footer
          data-component="StockAnalysisFooter"
          className="flex items-center justify-between text-[11px] text-[var(--pellucid-muted)]"
        >
          <span data-field="trend">
            Trend: {out.response.metrics.trend} ({out.response.metrics.magnitude})
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
  if (out.code === "entitlement_forbidden") {
    return (
      <div
        role="alert"
        data-state="entitlement"
        data-error-code={out.code}
        className="text-xs text-[var(--pellucid-warn)]"
      >
        <strong>Premium tier required.</strong>
        <br />
        Stock analysis is part of the API / Business plan.
      </div>
    );
  }
  if (
    out.code === "bootstrap_upstream_empty" ||
    out.code === "entitlement_upstream_down"
  ) {
    return (
      <div
        role="alert"
        data-state="outage"
        data-error-code={out.code}
        className="text-xs text-[var(--pellucid-warn)]"
      >
        <strong>Pipeline offline.</strong>
        <br />
        {out.retryAfterSecs ? (
          <>Retry available in {out.retryAfterSecs}s.</>
        ) : (
          <>Retry shortly.</>
        )}
      </div>
    );
  }
  if (out.code === "symbol_not_found") {
    return (
      <div
        role="alert"
        data-state="not-found"
        className="text-xs text-[var(--pellucid-muted)]"
      >
        No data for that symbol in the current basket.
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

/**
 * Build the 4-tile metric grid from an analyze-stock response.
 * Pure — exported so unit tests pin the tile order + tones.
 */
export function metricTiles(r: AnalyzeStockResponse): MetricTile[] {
  const tone =
    r.metrics.trend === "up"
      ? "positive"
      : r.metrics.trend === "down"
        ? "negative"
        : "neutral";
  return [
    {
      id: "price",
      label: "Price",
      value: formatPrice(r.price, r.currency),
    },
    {
      id: "previous-close",
      label: "Prev close",
      value: formatPrice(r.previousClose, r.currency),
    },
    {
      id: "dollar-change",
      label: "$ Change",
      value: formatDollar(r.metrics.dollarChange, r.currency),
      tone,
    },
    {
      id: "percent-change",
      label: "% Change",
      value: formatPercent(r.metrics.percentChange),
      subline: `${r.metrics.trend} · ${r.metrics.magnitude}`,
      tone,
    },
  ];
}

function formatPrice(value: number, currency: string): string {
  if (!Number.isFinite(value)) return "—";
  const formatted = value.toLocaleString("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
  return `${formatted} ${currency}`;
}

function formatDollar(value: number, currency: string): string {
  if (!Number.isFinite(value)) return "—";
  const sign = value < 0 ? "-" : "+";
  const formatted = Math.abs(value).toLocaleString("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
  return `${sign}${formatted} ${currency}`;
}

function formatPercent(value: number): string {
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
    case "clerk_unauthorized":
      return "Sign-in required";
    case "network":
      return "Network error";
    default:
      return "Error";
  }
}

// Register in the family registry on import.
registerPanel({
  id: STOCK_ANALYSIS_PANEL_ID,
  title: "Stock analysis",
  blurb: "Per-symbol price + trend analytics (Premium / API).",
  component: StockAnalysisPanel as React.ComponentType<unknown>,
  cacheKeys: ["market:stocks-bootstrap:v1"],
  minTier: REQUIRED_TIER as 2,
  variants: "*",
});
