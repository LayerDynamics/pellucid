import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadEscalationCorrelation, type EscalationCorrelationResponse } from "../../data/loaders/geo/escalation-correlation";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const ESCALATION_CORRELATION_PANEL_ID = "geo/escalation-correlation";
export const REQUIRED_TIER = 0;

export interface EscalationCorrelationPanelProps {
  load?: typeof loadEscalationCorrelation;
}

export function EscalationCorrelationPanel(props: EscalationCorrelationPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(ESCALATION_CORRELATION_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadEscalationCorrelation, [props.load]);
  const view = usePanelLoad<EscalationCorrelationResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(ESCALATION_CORRELATION_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={ESCALATION_CORRELATION_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Escalation correlation">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Escalation correlation</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function escalationClass(e: number): string {
  if (e >= 60) return "text-[var(--pellucid-danger)]";
  if (e >= 30) return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<EscalationCorrelationResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Escalation feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No correlated zones.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} zones</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} escalation rows`}>
        {r.rows.map((row) => (
          <li
            key={row.zone}
            data-component="EscalationRow"
            data-zone={row.zone}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono">{row.zone}</span>
            <span data-field="thermal-anomalies" className="font-mono text-[var(--pellucid-muted)]">{row.thermalAnomalies} 🔥</span>
            <span data-field="fatalities-24h" className="font-mono text-[var(--pellucid-muted)]">{row.fatalities24h} k</span>
            <span data-field="escalation" className={`ml-2 font-mono ${escalationClass(row.escalation)}`}>{row.escalation}/100</span>
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
  id: ESCALATION_CORRELATION_PANEL_ID,
  title: "Escalation correlation",
  blurb: "Thermal anomalies × UCDP fatalities, by zone.",
  component: EscalationCorrelationPanel as React.ComponentType<unknown>,
  cacheKeys: ["thermal:anomaly-feed:current:v1", "conflict:events-24h:v1"],
  minTier: 0,
  variants: "*",
});
