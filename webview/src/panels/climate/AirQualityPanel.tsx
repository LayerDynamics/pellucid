import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadAirQuality, type AirQualityResponse } from "../../data/loaders/climate/air-quality";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const AIR_QUALITY_PANEL_ID = "climate/air-quality";
export const REQUIRED_TIER = 0;

export interface AirQualityPanelProps {
  load?: typeof loadAirQuality;
}

export function AirQualityPanel(props: AirQualityPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(AIR_QUALITY_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadAirQuality, [props.load]);
  const view = usePanelLoad<AirQualityResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(AIR_QUALITY_PANEL_ID, { rowSpan: 2, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={AIR_QUALITY_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Air quality">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Air quality</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function aqiClass(aqi: number): string {
  if (aqi >= 150) return "text-[var(--pellucid-danger)]";
  if (aqi >= 100) return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-success)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<AirQualityResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading AQI…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>OpenAQ pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No AQI data.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">Worst AQI: <span data-field="worst-aqi" className={`font-mono ${aqiClass(r.worstAqi)}`}>{r.worstAqi}</span></div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} cities`}>
        {r.rows.map((row, i) => (
          <li
            key={`${row.city}-${i}`}
            data-component="AirQualityRow"
            data-city={row.city}
            data-tone={row.aqi >= 150 ? "negative" : row.aqi >= 100 ? "warn" : "positive"}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono">{row.city}</span>
            <span className="text-[var(--pellucid-muted)]">{row.country}</span>
            <span data-field="aqi" className={`ml-2 font-mono ${aqiClass(row.aqi)}`}>{row.aqi}</span>
            <span data-field="pollutant" className="ml-2 font-mono text-[var(--pellucid-muted)]">{row.pollutant}</span>
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
  id: AIR_QUALITY_PANEL_ID,
  title: "Air quality",
  blurb: "OpenAQ AQI snapshot per city.",
  component: AirQualityPanel as React.ComponentType<unknown>,
  cacheKeys: ["climate:air-quality:current:v1"],
  minTier: 0,
  variants: "*",
});
