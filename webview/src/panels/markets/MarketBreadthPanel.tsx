import { useEffect, useState, type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadBreadth,
  type BreadthOutcome,
  type BreadthQuery,
  type BreadthResponse,
  type BreadthRow,
} from "../../data/loaders/market/breadth";
import {
  MetricGrid,
  registerPanel,
  type MetricTile,
} from "./index";

/** Stable id for `usePanelStore` registration. */
export const MARKET_BREADTH_PANEL_ID = "markets/breadth";

/** Anonymous-tier handler. The panel mirrors the constant for
 *  forward compat. */
export const REQUIRED_TIER = 0;

/** Default `topN` when the caller doesn't override. */
export const DEFAULT_TOP_N = 5;

export interface MarketBreadthPanelProps {
  /** Optional query (top-N override). */
  query?: BreadthQuery;
  /** Override the loader (testing). */
  load?: typeof loadBreadth;
  /** Optional bearer token forwarded to the loader. */
  bearerToken?: string;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: BreadthOutcome };

/**
 * Market breadth panel — M3 family 4.2, T4.2.4.
 *
 * Renders the advance/decline counts + new-high / new-low
 * counts derived from the cached basket, plus two top-N tables
 * (top advancers + top decliners). The user can dial the table
 * size through a slider.
 */
export function MarketBreadthPanel(
  props: MarketBreadthPanelProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(MARKET_BREADTH_PANEL_ID));
  const initialTopN = props.query?.topN ?? DEFAULT_TOP_N;
  const [topN, setTopN] = useState<number>(initialTopN);
  const [view, setView] = useState<ViewState>({ kind: "loading" });

  useEffect(() => {
    setLayout(MARKET_BREADTH_PANEL_ID, { rowSpan: 2, colSpan: 2 });
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
    const loader = props.load ?? loadBreadth;
    void loader(
      { topN },
      props.bearerToken ? { bearerToken: props.bearerToken } : {},
    ).then((outcome) => {
      if (cancelled) return;
      setView({ kind: "ready", outcome });
    });
    return () => {
      cancelled = true;
    };
  }, [hasTier, topN, props.load, props.bearerToken]);

  if (isHidden) return <></>;

  return (
    <section
      data-panel-id={MARKET_BREADTH_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Market breadth"
    >
      <header className="flex items-baseline justify-between gap-3">
        <h3 className="text-sm font-semibold">Market breadth</h3>
        <TopNControl current={topN} onChange={setTopN} />
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ViewState): ReactElement {
  if (view.kind === "loading") {
    return (
      <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">
        Computing breadth…
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
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") {
      return (
        <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]">
          <strong>Markets pipeline offline.</strong>
          {out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}
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
  return <BreadthBody response={out.response} />;
}

function BreadthBody({ response }: { response: BreadthResponse }): ReactElement {
  const tiles: MetricTile[] = [
    {
      id: "advancers",
      label: "Advancers",
      value: String(response.advancers),
      tone: "positive",
    },
    {
      id: "decliners",
      label: "Decliners",
      value: String(response.decliners),
      tone: "negative",
    },
    {
      id: "ad-line",
      label: "A/D line",
      value: formatSignedInt(response.advanceDeclineLine),
      tone:
        response.advanceDeclineLine > 0
          ? "positive"
          : response.advanceDeclineLine < 0
          ? "negative"
          : "neutral",
    },
    {
      id: "new-highs",
      label: "New highs",
      value: String(response.newHighs),
      tone: "positive",
    },
    {
      id: "new-lows",
      label: "New lows",
      value: String(response.newLows),
      tone: "negative",
    },
    {
      id: "unchanged",
      label: "Unchanged",
      value: String(response.unchanged),
      tone: "neutral",
    },
  ];
  return (
    <div data-state="ready" className="flex flex-col gap-3">
      <MetricGrid tiles={tiles} />
      <div className="grid grid-cols-2 gap-3">
        <TopTable
          title="Top advancers"
          rows={response.topAdvancers}
          tone="positive"
          dataAttr="top-advancers"
        />
        <TopTable
          title="Top decliners"
          rows={response.topDecliners}
          tone="negative"
          dataAttr="top-decliners"
        />
      </div>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        Universe: {response.universe} symbols · as of{" "}
        {new Date(response.assembledAtMs).toUTCString()}
        {response.stale ? (
          <>
            {" "}
            · <span data-field="stale">cached snapshot</span>
          </>
        ) : null}
      </footer>
    </div>
  );
}

function TopTable({
  title,
  rows,
  tone,
  dataAttr,
}: {
  title: string;
  rows: BreadthRow[];
  tone: "positive" | "negative";
  dataAttr: string;
}): ReactElement {
  return (
    <article
      data-component="TopTable"
      data-table={dataAttr}
      className="rounded border border-[var(--pellucid-border)] p-2"
    >
      <h4 className="mb-1 text-[11px] font-semibold uppercase tracking-wide">
        {title}
      </h4>
      {rows.length === 0 ? (
        <div className="text-[11px] text-[var(--pellucid-muted)]">
          No symbols matched.
        </div>
      ) : (
        <table className="w-full text-[11px]">
          <tbody>
            {rows.map((r) => (
              <tr key={r.symbol} data-row-symbol={r.symbol}>
                <td className="text-left font-mono">{r.symbol}</td>
                <td
                  className={`text-right font-mono ${
                    tone === "positive"
                      ? "text-[var(--pellucid-up)]"
                      : "text-[var(--pellucid-down)]"
                  }`}
                >
                  {formatPercent(r.percentChange)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </article>
  );
}

function TopNControl({
  current,
  onChange,
}: {
  current: number;
  onChange: (next: number) => void;
}): ReactElement {
  return (
    <label className="flex items-center gap-1 text-[11px]">
      <span className="text-[var(--pellucid-muted)]">Top N</span>
      <input
        type="number"
        min={1}
        max={25}
        value={current}
        aria-label="Top N"
        data-field="topn-input"
        className="w-14 rounded border border-[var(--pellucid-border)] bg-transparent px-1 py-0.5 text-[var(--pellucid-fg)]"
        onChange={(e) => {
          const parsed = Number.parseInt(e.target.value, 10);
          if (Number.isFinite(parsed) && parsed > 0)
            onChange(Math.min(25, Math.max(1, parsed)));
        }}
      />
    </label>
  );
}

function formatSignedInt(value: number): string {
  if (value > 0) return `+${value}`;
  return String(value);
}

function formatPercent(value: number): string {
  const sign = value > 0 ? "+" : value < 0 ? "" : "±";
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
    case "network":
      return "Network error";
    default:
      return "Error";
  }
}

registerPanel({
  id: MARKET_BREADTH_PANEL_ID,
  title: "Market breadth",
  blurb: "Advance/decline counts, new highs / new lows, top movers.",
  component: MarketBreadthPanel as React.ComponentType<unknown>,
  cacheKeys: ["market:stocks-bootstrap:v1"],
  minTier: 0,
  variants: "*",
});
