import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadPredictionMarkets, type PredictionMarketsResponse } from "../../data/loaders/forecast/prediction-markets";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const PREDICTION_MARKETS_PANEL_ID = "forecast/prediction-markets";
export const REQUIRED_TIER = 0;

export interface PredictionMarketsPanelProps {
  load?: typeof loadPredictionMarkets;
}

export function PredictionMarketsPanel(props: PredictionMarketsPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(PREDICTION_MARKETS_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadPredictionMarkets, [props.load]);
  const view = usePanelLoad<PredictionMarketsResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(PREDICTION_MARKETS_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={PREDICTION_MARKETS_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Prediction markets">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Prediction markets</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<PredictionMarketsResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading markets…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Polymarket feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No active markets.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} active</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} markets`}>
        {r.rows.map((row) => (
          <li
            key={row.id}
            data-component="MarketRow"
            data-id={row.id}
            data-category={row.category}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="flex-1 truncate">{row.question}</span>
            <span data-field="yes-price" className="ml-2 font-mono">{(row.yes_price * 100).toFixed(0)}¢</span>
            <span data-field="volume-usd" className="ml-2 font-mono text-[var(--pellucid-muted)]">${row.volume_usd.toLocaleString()}</span>
            <span data-field="category" className="ml-2 font-mono uppercase text-[10px]">{row.category}</span>
          </li>
        ))}
      </ul>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        Snapshot: {formatAssembledAtUtc(r.assembledAtMs)}
        {r.stale ? <> · <span data-field="stale">cached</span></> : null}
      </footer>
    </div>
  );
}

registerPanel({
  id: PREDICTION_MARKETS_PANEL_ID,
  title: "Prediction markets",
  blurb: "Polymarket scenarios with YES price + volume.",
  component: PredictionMarketsPanel as React.ComponentType<unknown>,
  cacheKeys: ["prediction:scenario-state:current:v1"],
  minTier: 0,
  variants: "*",
});
