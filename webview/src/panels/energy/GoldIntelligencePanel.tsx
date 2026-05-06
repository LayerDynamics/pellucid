import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadGoldIntelligence, type GoldIntelligenceResponse } from "../../data/loaders/energy/gold-intelligence";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const GOLD_INTELLIGENCE_PANEL_ID = "commodities/gold-intelligence";
export const REQUIRED_TIER = 0;

export interface GoldIntelligencePanelProps {
  load?: typeof loadGoldIntelligence;
}

export function GoldIntelligencePanel(props: GoldIntelligencePanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(GOLD_INTELLIGENCE_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadGoldIntelligence, [props.load]);
  const view = usePanelLoad<GoldIntelligenceResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(GOLD_INTELLIGENCE_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section
      data-panel-id={GOLD_INTELLIGENCE_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Gold intelligence"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Gold intelligence</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<GoldIntelligenceResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading gold flows…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Gold pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No ETF flow data.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <div data-component="GoldTotalsTile" data-field="total-net-flow" className="rounded border border-[var(--pellucid-border)] p-2">
          <div className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">Net flow</div>
          <div className={`font-mono ${r.totalNetFlowMillionUsd >= 0 ? "text-[var(--pellucid-success)]" : "text-[var(--pellucid-danger)]"}`}>
            {r.totalNetFlowMillionUsd >= 0 ? "+" : ""}${r.totalNetFlowMillionUsd.toFixed(1)}M
          </div>
        </div>
        <div data-component="GoldTotalsTile" data-field="total-tonne" className="rounded border border-[var(--pellucid-border)] p-2">
          <div className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">Holdings</div>
          <div className="font-mono">{r.totalTonneHoldings.toFixed(0)} t</div>
        </div>
      </div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} gold ETF rows`}>
        {r.rows.map((row) => (
          <li
            key={row.ticker}
            data-component="GoldEtfRow"
            data-ticker={row.ticker}
            data-tone={row.netFlowMillionUsd >= 0 ? "positive" : "negative"}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono uppercase">{row.ticker}</span>
            <span className="text-[var(--pellucid-muted)]">{row.region}</span>
            <span
              data-field="net-flow"
              className={`ml-2 font-mono ${row.netFlowMillionUsd >= 0 ? "text-[var(--pellucid-success)]" : "text-[var(--pellucid-danger)]"}`}
            >
              {row.netFlowMillionUsd >= 0 ? "+" : ""}${row.netFlowMillionUsd.toFixed(1)}M
            </span>
            <span data-field="tonne" className="ml-2 font-mono">{row.tonneHoldings.toFixed(0)} t</span>
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
  id: GOLD_INTELLIGENCE_PANEL_ID,
  title: "Gold intelligence",
  blurb: "Gold ETF flow + tonne holdings.",
  component: GoldIntelligencePanel as React.ComponentType<unknown>,
  cacheKeys: ["market:gold-etf-flows:current:v1"],
  minTier: 0,
  variants: "*",
});
