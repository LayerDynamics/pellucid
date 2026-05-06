import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadCyberIncidents, type CyberIncidentsResponse } from "../../data/loaders/infra/cyber-incidents";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const CYBER_INCIDENTS_PANEL_ID = "infra/cyber-incidents";
export const REQUIRED_TIER = 0;

export interface CyberIncidentsPanelProps {
  load?: typeof loadCyberIncidents;
}

export function CyberIncidentsPanel(props: CyberIncidentsPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(CYBER_INCIDENTS_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadCyberIncidents, [props.load]);
  const view = usePanelLoad<CyberIncidentsResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(CYBER_INCIDENTS_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={CYBER_INCIDENTS_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Cyber incidents">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Cyber incidents (24 h)</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function severityClass(s: string): string {
  const sl = s.toLowerCase();
  if (sl === "critical" || sl === "extreme") return "text-[var(--pellucid-danger)]";
  if (sl === "high") return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<CyberIncidentsResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Cyber feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No incidents.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} incidents</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} incidents`}>
        {r.rows.map((row) => (
          <li
            key={row.id}
            data-component="IncidentRow"
            data-id={row.id}
            data-severity={row.severity}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span data-field="severity" className={`font-mono uppercase ${severityClass(row.severity)}`}>{row.severity}</span>
            <span className="flex-1 px-2 truncate">{row.title}</span>
            <span data-field="source" className="font-mono text-[var(--pellucid-muted)]">{row.source}</span>
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
  id: CYBER_INCIDENTS_PANEL_ID,
  title: "Cyber incidents (24h)",
  blurb: "Aggregated cyber incident feed.",
  component: CyberIncidentsPanel as React.ComponentType<unknown>,
  cacheKeys: ["cyber:incident-feed:24h:v1"],
  minTier: 0,
  variants: "*",
});
