import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadEarthquakes, type EarthquakesResponse } from "../../data/loaders/climate/earthquakes";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const EARTHQUAKES_PANEL_ID = "climate/earthquakes";
export const REQUIRED_TIER = 0;

export interface EarthquakesPanelProps {
  load?: typeof loadEarthquakes;
}

export function EarthquakesPanel(props: EarthquakesPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(EARTHQUAKES_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadEarthquakes, [props.load]);
  const view = usePanelLoad<EarthquakesResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(EARTHQUAKES_PANEL_ID, { rowSpan: 2, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={EARTHQUAKES_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Recent earthquakes">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Recent earthquakes</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function severityClass(mag: number): string {
  if (mag >= 6) return "text-[var(--pellucid-danger)]";
  if (mag >= 5) return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<EarthquakesResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading quakes…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>USGS pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No recent quakes.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">
        {r.total} in 24h · max <span data-field="max-mag" className="font-mono">M{r.maxMag.toFixed(1)}</span>
      </div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} quakes`}>
        {r.rows.map((row) => (
          <li
            key={row.id}
            data-component="QuakeRow"
            data-id={row.id}
            data-tone={row.mag >= 6 ? "negative" : "neutral"}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span data-field="mag" className={`font-mono ${severityClass(row.mag)}`}>M{row.mag.toFixed(1)}</span>
            <span className="flex-1 px-2 truncate">{row.place}</span>
            <span data-field="depth" className="font-mono text-[var(--pellucid-muted)]">{row.depth.toFixed(0)} km</span>
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
  id: EARTHQUAKES_PANEL_ID,
  title: "Recent earthquakes",
  blurb: "USGS 24-hour quake feed sorted by magnitude.",
  component: EarthquakesPanel as React.ComponentType<unknown>,
  cacheKeys: ["seismology:recent-quakes:24h:v1"],
  minTier: 0,
  variants: "*",
});
