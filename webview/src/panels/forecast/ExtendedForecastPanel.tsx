import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadExtendedForecast, type ExtendedResponse } from "../../data/loaders/forecast/extended";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const EXTENDED_FORECAST_PANEL_ID = "forecast/extended";
export const REQUIRED_TIER = 0;

export interface ExtendedForecastPanelProps {
  load?: typeof loadExtendedForecast;
}

export function ExtendedForecastPanel(props: ExtendedForecastPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(EXTENDED_FORECAST_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadExtendedForecast, [props.load]);
  const view = usePanelLoad<ExtendedResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(EXTENDED_FORECAST_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={EXTENDED_FORECAST_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Extended forecast">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Extended forecast</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<ExtendedResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Extended feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No extended forecasts.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} forecasts (sorted by |Δ7d|)</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} extended forecasts`}>
        {r.rows.map((row) => (
          <li
            key={row.id}
            data-component="ExtendedRow"
            data-id={row.id}
            data-tone={row.delta7d >= 0 ? "positive" : "negative"}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span data-field="horizon" className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">{row.horizon}</span>
            <span className="flex-1 px-2 truncate">{row.question}</span>
            <span data-field="probability" className="font-mono">{(row.probability * 100).toFixed(0)}%</span>
            <span
              data-field="delta-7d"
              className={`ml-2 font-mono ${row.delta7d >= 0 ? "text-[var(--pellucid-success)]" : "text-[var(--pellucid-danger)]"}`}
            >
              {row.delta7d >= 0 ? "+" : ""}{(row.delta7d * 100).toFixed(1)}pp
            </span>
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
  id: EXTENDED_FORECAST_PANEL_ID,
  title: "Extended forecast",
  blurb: "Extended-horizon forecasts with 7-day momentum.",
  component: ExtendedForecastPanel as React.ComponentType<unknown>,
  cacheKeys: ["forecast:extended:weekly:v1"],
  minTier: 0,
  variants: "*",
});
