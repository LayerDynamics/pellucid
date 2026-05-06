import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadClimateAnomalies, type AnomaliesResponse } from "../../data/loaders/climate/anomalies";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const CLIMATE_ANOMALIES_PANEL_ID = "climate/anomalies";
export const REQUIRED_TIER = 0;

export interface ClimateAnomaliesPanelProps {
  load?: typeof loadClimateAnomalies;
}

export function ClimateAnomaliesPanel(props: ClimateAnomaliesPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(CLIMATE_ANOMALIES_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadClimateAnomalies, [props.load]);
  const view = usePanelLoad<AnomaliesResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(CLIMATE_ANOMALIES_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={CLIMATE_ANOMALIES_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Climate anomalies">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Climate anomalies</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function anomalyClass(c?: number): string {
  if (typeof c !== "number") return "text-[var(--pellucid-muted)]";
  if (c >= 1) return "text-[var(--pellucid-danger)]";
  if (c <= -0.5) return "text-[var(--pellucid-success)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<AnomaliesResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading anomalies…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Anomaly pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">
        Global anomaly:{" "}
        <span data-field="global-anomaly" className={`font-mono ${anomalyClass(r.globalAnomalyC)}`}>
          {typeof r.globalAnomalyC === "number" ? `${r.globalAnomalyC >= 0 ? "+" : ""}${r.globalAnomalyC.toFixed(2)} °C` : "—"}
        </span>
        {r.period ? <span className="ml-2 text-[var(--pellucid-muted)]">{r.period}</span> : null}
      </div>
      {r.records.length > 0 ? (
        <ul className="flex flex-col gap-1" aria-label={`${r.records.length} station records`}>
          {r.records.map((rec) => (
            <li
              key={rec.stationId}
              data-component="StationRecordRow"
              data-station={rec.stationId}
              data-class={rec.recordClass}
              className="flex items-baseline justify-between text-[11px]"
            >
              <span className="font-mono">{rec.stationId}</span>
              <span className="flex-1 px-2 truncate">{rec.label}</span>
              <span className="font-mono uppercase text-[var(--pellucid-muted)]">{rec.recordClass}</span>
              <span data-field="value" className="ml-2 font-mono">{rec.value.toFixed(1)}</span>
              <span data-field="set-on" className="ml-2 font-mono text-[var(--pellucid-muted)]">{rec.setOn}</span>
            </li>
          ))}
        </ul>
      ) : (
        <div className="text-[11px] text-[var(--pellucid-muted)]">No station records.</div>
      )}
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        Snapshot: {formatAssembledAtUtc(r.assembledAtMs)}
        {r.stale ? <> · <span data-field="stale">cached</span></> : null}
      </footer>
    </div>
  );
}

registerPanel({
  id: CLIMATE_ANOMALIES_PANEL_ID,
  title: "Climate anomalies",
  blurb: "Global temp anomaly + station-record list.",
  component: ClimateAnomaliesPanel as React.ComponentType<unknown>,
  cacheKeys: ["climate:latest-anomaly:global:v1", "climate:station-records:monthly:v1"],
  minTier: 0,
  variants: "*",
});
