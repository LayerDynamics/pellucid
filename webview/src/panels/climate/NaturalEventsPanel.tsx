import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadNaturalEvents, type NaturalEventsResponse } from "../../data/loaders/climate/natural-events";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const NATURAL_EVENTS_PANEL_ID = "climate/natural-events";
export const REQUIRED_TIER = 0;

export interface NaturalEventsPanelProps {
  load?: typeof loadNaturalEvents;
}

export function NaturalEventsPanel(props: NaturalEventsPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(NATURAL_EVENTS_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadNaturalEvents, [props.load]);
  const view = usePanelLoad<NaturalEventsResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(NATURAL_EVENTS_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={NATURAL_EVENTS_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Natural events">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Natural events</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function severityClass(s: string): string {
  if (s === "extreme") return "text-[var(--pellucid-danger)]";
  if (s === "severe") return "text-[var(--pellucid-warn)]";
  if (s === "moderate") return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<NaturalEventsResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading natural events…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Natural-events pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.totalEvents} events</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.tiles.length} natural events`}>
        {r.tiles.map((t, i) => (
          <li
            key={`${t.kind}-${t.label}-${i}`}
            data-component="EventTile"
            data-kind={t.kind}
            data-severity={t.severity}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span data-field="kind" className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">{t.kind}</span>
            <span className="flex-1 px-2 truncate">{t.label}</span>
            <span className="font-mono text-[var(--pellucid-muted)]">{t.region}</span>
            <span data-field="severity" className={`ml-2 font-mono uppercase ${severityClass(t.severity)}`}>{t.severity}</span>
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
  id: NATURAL_EVENTS_PANEL_ID,
  title: "Natural events",
  blurb: "Unified volcano + wildfire + quake feed.",
  component: NaturalEventsPanel as React.ComponentType<unknown>,
  cacheKeys: ["natural:volcano-feed:current:v1", "wildfire:active-perimeters:current:v1", "seismology:recent-quakes:24h:v1"],
  minTier: 0,
  variants: "*",
});
