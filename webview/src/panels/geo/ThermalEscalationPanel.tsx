import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadThermalEscalation, type ThermalEscalationResponse } from "../../data/loaders/geo/thermal-escalation";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const THERMAL_ESCALATION_PANEL_ID = "geo/thermal-escalation";
export const REQUIRED_TIER = 0;

export interface ThermalEscalationPanelProps {
  load?: typeof loadThermalEscalation;
}

export function ThermalEscalationPanel(props: ThermalEscalationPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(THERMAL_ESCALATION_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadThermalEscalation, [props.load]);
  const view = usePanelLoad<ThermalEscalationResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(THERMAL_ESCALATION_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={THERMAL_ESCALATION_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Thermal escalation">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Thermal escalation</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function brightnessClass(k: number): string {
  if (k >= 400) return "text-[var(--pellucid-danger)]";
  if (k >= 360) return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<ThermalEscalationResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading thermal…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Thermal feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No thermal anomalies.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">
        {r.total} anomalies · <span data-field="high-confidence-count" className="font-mono">{r.highConfidenceCount}</span> high confidence
      </div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} thermal anomalies`}>
        {r.rows.map((row) => (
          <li
            key={row.id}
            data-component="ThermalRow"
            data-id={row.id}
            data-zone={row.zone}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span data-field="zone" className="font-mono uppercase">{row.zone}</span>
            <span className="text-[var(--pellucid-muted)]">{row.lat.toFixed(2)}, {row.lon.toFixed(2)}</span>
            <span data-field="brightness-k" className={`font-mono ${brightnessClass(row.brightnessK)}`}>{row.brightnessK.toFixed(0)} K</span>
            <span data-field="confidence" className="ml-2 font-mono text-[var(--pellucid-muted)]">{row.confidence}%</span>
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
  id: THERMAL_ESCALATION_PANEL_ID,
  title: "Thermal escalation",
  blurb: "MODIS / VIIRS thermal anomalies in conflict zones.",
  component: ThermalEscalationPanel as React.ComponentType<unknown>,
  cacheKeys: ["thermal:anomaly-feed:current:v1"],
  minTier: 0,
  variants: "*",
});
