import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadMilitaryCorrelation, type MilitaryCorrelationResponse } from "../../data/loaders/geo/military-correlation";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const MILITARY_CORRELATION_PANEL_ID = "geo/military-correlation";
export const REQUIRED_TIER = 0;

export interface MilitaryCorrelationPanelProps {
  load?: typeof loadMilitaryCorrelation;
}

export function MilitaryCorrelationPanel(props: MilitaryCorrelationPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(MILITARY_CORRELATION_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadMilitaryCorrelation, [props.load]);
  const view = usePanelLoad<MilitaryCorrelationResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(MILITARY_CORRELATION_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={MILITARY_CORRELATION_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Military correlation">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Military correlation</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function correlationClass(c: number): string {
  if (c >= 60) return "text-[var(--pellucid-danger)]";
  if (c >= 30) return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<MilitaryCorrelationResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading correlation…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Correlation feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No correlations.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} theaters</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} correlation rows`}>
        {r.rows.map((row, i) => (
          <li
            key={`${row.theater}-${i}`}
            data-component="CorrelationRow"
            data-theater={row.theater}
            className="flex flex-col gap-0.5 text-[11px] rounded border border-[var(--pellucid-border)] p-1.5"
          >
            <div className="flex items-baseline justify-between">
              <span className="font-mono uppercase">{row.theater}</span>
              <span data-field="readiness" className="font-mono uppercase">{row.readiness}</span>
              <span data-field="correlation" className={`font-mono ${correlationClass(row.correlation)}`}>{row.correlation}/100</span>
            </div>
            <div className="flex justify-between text-[10px] text-[var(--pellucid-muted)]">
              <span data-field="headcount">{row.headcount.toLocaleString()} troops</span>
              <span data-field="fatalities-24h">{row.fatalities24h} k</span>
              <span data-field="deployments">{row.deployments} deployments</span>
            </div>
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
  id: MILITARY_CORRELATION_PANEL_ID,
  title: "Military correlation",
  blurb: "Per-theater readiness × UCDP × deployments score.",
  component: MilitaryCorrelationPanel as React.ComponentType<unknown>,
  cacheKeys: ["military:theater-posture:current:v1", "conflict:events-24h:v1", "military:active-deployments:v1"],
  minTier: 0,
  variants: "*",
});
