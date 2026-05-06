import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadNoaaAlerts, type NoaaAlertsResponse } from "../../data/loaders/climate/noaa-alerts";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const WEATHER_ALERTS_PANEL_ID = "climate/weather-alerts";
export const REQUIRED_TIER = 0;

export interface WeatherAlertsPanelProps {
  load?: typeof loadNoaaAlerts;
}

export function WeatherAlertsPanel(props: WeatherAlertsPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(WEATHER_ALERTS_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadNoaaAlerts, [props.load]);
  const view = usePanelLoad<NoaaAlertsResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(WEATHER_ALERTS_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={WEATHER_ALERTS_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Weather alerts">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">NOAA weather alerts</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function severityClass(s: string): string {
  if (s === "Extreme") return "text-[var(--pellucid-danger)]";
  if (s === "Severe") return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<NoaaAlertsResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading alerts…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>NOAA pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No active alerts.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} active alerts</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} alerts`}>
        {r.rows.map((row, i) => (
          <li
            key={`${row.event}-${row.area}-${i}`}
            data-component="AlertRow"
            data-severity={row.severity}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span data-field="event" className={`font-mono ${severityClass(row.severity)}`}>{row.event}</span>
            <span className="flex-1 px-2 truncate">{row.area}</span>
            <span data-field="urgency" className="font-mono text-[var(--pellucid-muted)]">{row.urgency}</span>
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
  id: WEATHER_ALERTS_PANEL_ID,
  title: "Weather alerts",
  blurb: "NOAA active weather alerts.",
  component: WeatherAlertsPanel as React.ComponentType<unknown>,
  cacheKeys: ["climate:noaa-alerts:current:v1"],
  minTier: 0,
  variants: "*",
});
