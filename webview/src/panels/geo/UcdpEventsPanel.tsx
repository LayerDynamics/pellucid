import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadUcdpEvents, type UcdpEventsResponse } from "../../data/loaders/geo/ucdp-events";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const UCDP_EVENTS_PANEL_ID = "geo/ucdp-events";
export const REQUIRED_TIER = 0;

export interface UcdpEventsPanelProps {
  load?: typeof loadUcdpEvents;
}

export function UcdpEventsPanel(props: UcdpEventsPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(UCDP_EVENTS_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadUcdpEvents, [props.load]);
  const view = usePanelLoad<UcdpEventsResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(UCDP_EVENTS_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section
      data-panel-id={UCDP_EVENTS_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="UCDP events (24 h)"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">UCDP events (24 h)</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function fatalitiesClass(n: number): string {
  if (n >= 50) return "text-[var(--pellucid-danger)]";
  if (n >= 10) return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<UcdpEventsResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading UCDP…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>UCDP feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No events in the last 24 h.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">
        {r.total} events · total fatalities <span data-field="total-fatalities" className="font-mono">{r.totalFatalities}</span>
      </div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} UCDP events`}>
        {r.rows.map((row) => (
          <li
            key={row.id}
            data-component="UcdpEventRow"
            data-id={row.id}
            data-country={row.country}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono uppercase">{row.country}</span>
            <span className="flex-1 px-2 truncate">{row.actor1} vs {row.actor2}</span>
            <span data-field="fatalities" className={`font-mono ${fatalitiesClass(row.fatalities)}`}>{row.fatalities} k</span>
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
  id: UCDP_EVENTS_PANEL_ID,
  title: "UCDP events (24 h)",
  blurb: "Latest UCDP GED conflict events with fatality count.",
  component: UcdpEventsPanel as React.ComponentType<unknown>,
  cacheKeys: ["conflict:events-24h:v1"],
  minTier: 0,
  variants: "*",
});
