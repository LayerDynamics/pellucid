import { useEffect, useState, type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadEtfFlows,
  type EtfFlowsOutcome,
  type EtfFlowsResponse,
  type EtfFlowsQuery,
  type SortMode,
} from "../../data/loaders/market/etf-flows";
import { registerPanel } from "./index";

/** Stable id for `usePanelStore` registration. */
export const ETF_FLOWS_PANEL_ID = "markets/etf-flows";

/** Anonymous handler. */
export const REQUIRED_TIER = 0;

/** Default initial sort. */
export const DEFAULT_SORT: SortMode = "activity-ratio-desc";

const SORT_LABELS: Record<SortMode, string> = {
  "activity-ratio-desc": "Activity (high → low)",
  "activity-ratio-asc": "Activity (low → high)",
  "dollar-volume-desc": "Dollar volume (largest)",
  "symbol-asc": "Symbol (A → Z)",
};

const SORT_OPTIONS: SortMode[] = [
  "activity-ratio-desc",
  "activity-ratio-asc",
  "dollar-volume-desc",
  "symbol-asc",
];

export interface ETFFlowsPanelProps {
  /** Optional pre-set query. */
  query?: EtfFlowsQuery;
  /** Override the loader (testing). */
  load?: typeof loadEtfFlows;
  /** Optional bearer token. */
  bearerToken?: string;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: EtfFlowsOutcome };

/**
 * ETF flows panel — M3 family 4.2, T4.2.5.
 *
 * Renders the seeder snapshot of dollar-volume / activity-ratio
 * across the ETF basket as a sortable table. The user can swap
 * the sort mode + filter by symbol.
 */
export function ETFFlowsPanel(props: ETFFlowsPanelProps): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(ETF_FLOWS_PANEL_ID));

  const [sort, setSort] = useState<SortMode>(props.query?.sort ?? DEFAULT_SORT);
  const [view, setView] = useState<ViewState>({ kind: "loading" });

  useEffect(() => {
    setLayout(ETF_FLOWS_PANEL_ID, { rowSpan: 2, colSpan: 2 });
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
    const loader = props.load ?? loadEtfFlows;
    const query: EtfFlowsQuery = { ...(props.query ?? {}), sort };
    void loader(
      query,
      props.bearerToken ? { bearerToken: props.bearerToken } : {},
    ).then((outcome) => {
      if (cancelled) return;
      setView({ kind: "ready", outcome });
    });
    return () => {
      cancelled = true;
    };
  }, [hasTier, sort, props.query, props.load, props.bearerToken]);

  if (isHidden) return <></>;

  return (
    <section
      data-panel-id={ETF_FLOWS_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="ETF flows"
    >
      <header className="flex items-baseline justify-between gap-3">
        <h3 className="text-sm font-semibold">ETF flows</h3>
        <SortControl current={sort} onChange={setSort} />
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ViewState): ReactElement {
  if (view.kind === "loading") {
    return (
      <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">
        Loading ETF flows…
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
          <strong>ETF flows pipeline offline.</strong>
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
  return <FlowsTable response={out.response} />;
}

function FlowsTable({ response }: { response: EtfFlowsResponse }): ReactElement {
  if (response.rows.length === 0) {
    return (
      <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">
        No rows match the current filter.
      </div>
    );
  }
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <table
        className="w-full text-[11px]"
        aria-label={`${response.rows.length} ETF flow rows`}
      >
        <thead className="text-[var(--pellucid-muted)]">
          <tr>
            <th className="text-left font-normal">Symbol</th>
            <th className="text-right font-normal">Latest $vol</th>
            <th className="text-right font-normal">
              Avg ($, {response.lookbackDays}d)
            </th>
            <th className="text-right font-normal">vs typical</th>
          </tr>
        </thead>
        <tbody>
          {response.rows.map((r) => (
            <tr key={r.symbol} data-row-symbol={r.symbol}>
              <td className="text-left font-mono">{r.symbol}</td>
              <td className="text-right font-mono">
                {formatDollar(r.latestDollarVolume)}
              </td>
              <td className="text-right font-mono">
                {formatDollar(r.avgDollarVolume)}
              </td>
              <td
                className={`text-right font-mono ${ratioToneClass(r.activityRatio)}`}
                data-field="activity-ratio"
              >
                {formatRatio(r.activityRatio)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        {response.rows.length} of {response.total} rows · as of{" "}
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

function SortControl({
  current,
  onChange,
}: {
  current: SortMode;
  onChange: (next: SortMode) => void;
}): ReactElement {
  return (
    <label className="flex items-center gap-1 text-[11px]">
      <span className="text-[var(--pellucid-muted)]">Sort</span>
      <select
        value={current}
        aria-label="Sort mode"
        data-field="sort-select"
        className="rounded border border-[var(--pellucid-border)] bg-transparent px-1 py-0.5 text-[var(--pellucid-fg)]"
        onChange={(e) => onChange(e.target.value as SortMode)}
      >
        {SORT_OPTIONS.map((mode) => (
          <option key={mode} value={mode}>
            {SORT_LABELS[mode]}
          </option>
        ))}
      </select>
    </label>
  );
}

function formatDollar(value: number): string {
  if (!Number.isFinite(value)) return "—";
  if (value >= 1_000_000_000) return `$${(value / 1_000_000_000).toFixed(2)}B`;
  if (value >= 1_000_000) return `$${(value / 1_000_000).toFixed(2)}M`;
  if (value >= 1_000) return `$${(value / 1_000).toFixed(2)}K`;
  return `$${value.toFixed(0)}`;
}

function formatRatio(value: number): string {
  if (!Number.isFinite(value) || value === 0) return "—";
  return `${value.toFixed(2)}×`;
}

function ratioToneClass(value: number): string {
  if (!Number.isFinite(value) || value === 0) return "";
  if (value > 1.25) return "text-[var(--pellucid-up)]";
  if (value < 0.75) return "text-[var(--pellucid-down)]";
  return "";
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
  id: ETF_FLOWS_PANEL_ID,
  title: "ETF flows",
  blurb: "Dollar-volume snapshot vs trailing-window average.",
  component: ETFFlowsPanel as React.ComponentType<unknown>,
  cacheKeys: ["market:etf-flows:current:v1"],
  minTier: 0,
  variants: "*",
});
