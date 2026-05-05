import {
  useEffect,
  useMemo,
  useState,
  type ReactElement,
} from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadBacktestStock,
  type BacktestStockOutcome,
  type BacktestStockResponse,
  type BacktestStockQuery,
  type Strategy,
  type StrategyResult,
} from "../../data/loaders/market/backtest-stock";
import {
  MetricGrid,
  registerPanel,
  type MetricTile,
} from "./index";

/** Stable id for `usePanelStore` registration. */
export const STOCK_BACKTEST_PANEL_ID = "markets/stock-backtest";

/** Tier gate. Mirrors the gateway-enforced REQUIRED_TIER on the
 *  Rust handler (tier 2). The client-side mirror lets the panel
 *  render a "locked" branch without a network round-trip. */
export const REQUIRED_TIER = 2;

/** Default initial limit on the universe size. */
export const DEFAULT_UNIVERSE_LIMIT = 50;

/** Strategy ids in the order the panel renders them. Matches the
 *  Rust handler's emission order so the user sees consistent
 *  columns regardless of map iteration semantics. */
export const STRATEGY_ORDER: Strategy[] = [
  "equal-weight",
  "momentum",
  "mean-reversion",
];

const STRATEGY_LABEL: Record<Strategy, string> = {
  "equal-weight": "Equal weight",
  momentum: "Momentum",
  "mean-reversion": "Mean reversion",
};

export interface StockBacktestPanelProps {
  /** Initial filter passed to the loader. */
  query?: BacktestStockQuery;
  /** Override the loader (testing). */
  load?: typeof loadBacktestStock;
  /** Optional bearer token forwarded to the loader. */
  bearerToken?: string;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: BacktestStockOutcome };

/**
 * Stock backtest panel — M3 family 4.2, T4.2.3.
 *
 * Tier-2 gated. Renders the three deterministic strategy
 * results the handler computes against the
 * `market:stocks-bootstrap:v1` snapshot: equal-weight,
 * momentum, mean-reversion. Each strategy gets its own metric
 * row (total return, win rate, max drawdown) plus a per-pick
 * table sorted by contribution magnitude.
 */
export function StockBacktestPanel(
  props: StockBacktestPanelProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(STOCK_BACKTEST_PANEL_ID));

  const initialLimit = props.query?.limitUniverse ?? DEFAULT_UNIVERSE_LIMIT;
  const initialSymbols = props.query?.symbols ?? "";
  const [limitUniverse, setLimitUniverse] = useState<number>(initialLimit);
  const [symbols, setSymbols] = useState<string>(initialSymbols);
  const [view, setView] = useState<ViewState>({ kind: "loading" });

  const effectiveQuery = useMemo<BacktestStockQuery>(() => {
    const q: BacktestStockQuery = { limitUniverse };
    const trimmed = symbols.trim();
    if (trimmed.length > 0) q.symbols = trimmed;
    return q;
  }, [limitUniverse, symbols]);

  useEffect(() => {
    setLayout(STOCK_BACKTEST_PANEL_ID, { rowSpan: 3, colSpan: 3 });
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
    const loader = props.load ?? loadBacktestStock;
    void loader(effectiveQuery, {
      ...(props.bearerToken ? { bearerToken: props.bearerToken } : {}),
    }).then((outcome) => {
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
      data-panel-id={STOCK_BACKTEST_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Stock backtest"
    >
      <header className="flex items-baseline justify-between gap-3">
        <h3 className="text-sm font-semibold">Stock backtest</h3>
        <UniverseControls
          limit={limitUniverse}
          symbols={symbols}
          onLimitChange={setLimitUniverse}
          onSymbolsChange={setSymbols}
        />
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ViewState): ReactElement {
  if (view.kind === "loading") {
    return (
      <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">
        Running backtest…
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
        Locked — backtest requires tier {view.minTier} or higher.
      </div>
    );
  }
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") {
      return (
        <div
          role="alert"
          data-state="outage"
          className="text-xs text-[var(--pellucid-warn)]"
        >
          <strong>Markets pipeline offline.</strong>
          {out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}
        </div>
      );
    }
    if (out.code === "empty_universe") {
      return (
        <div
          role="alert"
          data-state="empty-universe"
          className="text-xs text-[var(--pellucid-warn)]"
        >
          No symbols match the current filter.
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
  return <ResultsTable response={out.response} />;
}

function ResultsTable({ response }: { response: BacktestStockResponse }): ReactElement {
  const ordered = STRATEGY_ORDER.map((id) =>
    response.strategies.find((s) => s.strategy === id),
  ).filter((s): s is StrategyResult => s !== undefined);

  return (
    <div data-state="ready" className="flex flex-col gap-3">
      <div className="flex flex-wrap items-baseline gap-3 text-[11px] text-[var(--pellucid-muted)]">
        <span>
          Universe: <strong>{response.universe.length}</strong> symbols
        </span>
        <span>
          As of {new Date(response.assembledAtMs).toUTCString()}
        </span>
        {response.stale ? (
          <span data-field="stale">Showing cached snapshot.</span>
        ) : null}
      </div>
      {ordered.map((strategy) => (
        <StrategyCard key={strategy.strategy} result={strategy} />
      ))}
    </div>
  );
}

function StrategyCard({ result }: { result: StrategyResult }): ReactElement {
  const tiles: MetricTile[] = [
    {
      id: `${result.strategy}-total`,
      label: "Total return",
      value: formatPercent(result.metrics.totalReturnPct),
      tone: signTone(result.metrics.totalReturnPct),
    },
    {
      id: `${result.strategy}-winrate`,
      label: "Win rate",
      value: `${result.metrics.winRatePct.toFixed(0)}%`,
      tone: "neutral",
    },
    {
      id: `${result.strategy}-drawdown`,
      label: "Max drawdown",
      value: formatPercent(result.metrics.maxDrawdownPct),
      tone: result.metrics.maxDrawdownPct < 0 ? "negative" : "neutral",
    },
  ];
  const sortedPicks = [...result.picks].sort(
    (a, b) => Math.abs(b.contribution) - Math.abs(a.contribution),
  );

  return (
    <article
      data-component="StrategyCard"
      data-strategy={result.strategy}
      className="rounded border border-[var(--pellucid-border)] p-3"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h4 className="text-xs font-semibold uppercase tracking-wide">
          {STRATEGY_LABEL[result.strategy]}
        </h4>
        <span className="text-[10px] text-[var(--pellucid-muted)]">
          {result.picks.length} picks
        </span>
      </header>
      <MetricGrid tiles={tiles} />
      <table
        className="mt-2 w-full text-[11px]"
        aria-label={`${STRATEGY_LABEL[result.strategy]} picks`}
      >
        <thead className="text-[var(--pellucid-muted)]">
          <tr>
            <th className="text-left font-normal">Symbol</th>
            <th className="text-right font-normal">Weight</th>
            <th className="text-right font-normal">Δ%</th>
            <th className="text-right font-normal">Contribution</th>
          </tr>
        </thead>
        <tbody>
          {sortedPicks.map((p) => (
            <tr key={p.symbol} data-pick-symbol={p.symbol}>
              <td className="text-left font-mono">{p.symbol}</td>
              <td className="text-right font-mono">{p.weight.toFixed(2)}</td>
              <td
                className={`text-right font-mono ${signClass(p.percentChange)}`}
              >
                {formatPercent(p.percentChange)}
              </td>
              <td
                className={`text-right font-mono ${signClass(p.contribution)}`}
              >
                {formatPercent(p.contribution)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </article>
  );
}

function UniverseControls({
  limit,
  symbols,
  onLimitChange,
  onSymbolsChange,
}: {
  limit: number;
  symbols: string;
  onLimitChange: (next: number) => void;
  onSymbolsChange: (next: string) => void;
}): ReactElement {
  return (
    <div role="toolbar" aria-label="Universe filters" className="flex items-center gap-3 text-[11px]">
      <label className="flex items-center gap-1">
        <span className="text-[var(--pellucid-muted)]">Limit</span>
        <input
          type="number"
          min={1}
          max={50}
          value={limit}
          aria-label="Universe limit"
          data-field="limit-input"
          className="w-14 rounded border border-[var(--pellucid-border)] bg-transparent px-1 py-0.5 text-[var(--pellucid-fg)]"
          onChange={(e) => {
            const parsed = Number.parseInt(e.target.value, 10);
            if (Number.isFinite(parsed) && parsed > 0)
              onLimitChange(Math.min(50, Math.max(1, parsed)));
          }}
        />
      </label>
      <label className="flex items-center gap-1">
        <span className="text-[var(--pellucid-muted)]">Symbols</span>
        <input
          type="text"
          value={symbols}
          aria-label="Symbol allow-list (CSV)"
          data-field="symbols-input"
          placeholder="e.g. SPY,QQQ"
          className="w-32 rounded border border-[var(--pellucid-border)] bg-transparent px-1 py-0.5 text-[var(--pellucid-fg)]"
          onChange={(e) => onSymbolsChange(e.target.value)}
        />
      </label>
    </div>
  );
}

function formatPercent(value: number): string {
  const sign = value > 0 ? "+" : value < 0 ? "" : "±";
  return `${sign}${value.toFixed(2)}%`;
}

function signClass(value: number): string {
  if (value > 0) return "text-[var(--pellucid-up)]";
  if (value < 0) return "text-[var(--pellucid-down)]";
  return "";
}

function signTone(value: number): "positive" | "negative" | "neutral" {
  if (value > 0) return "positive";
  if (value < 0) return "negative";
  return "neutral";
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

registerPanel({
  id: STOCK_BACKTEST_PANEL_ID,
  title: "Stock backtest",
  blurb: "Three-strategy walk-forward backtest of today's market basket.",
  component: StockBacktestPanel as React.ComponentType<unknown>,
  cacheKeys: ["market:stocks-bootstrap:v1"],
  minTier: REQUIRED_TIER as 2,
  variants: "*",
});
