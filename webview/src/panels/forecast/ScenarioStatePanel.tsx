import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadScenarioState, type ScenarioStateResponse } from "../../data/loaders/forecast/scenario-state";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const SCENARIO_STATE_PANEL_ID = "forecast/scenario-state";
export const REQUIRED_TIER = 0;

export interface ScenarioStatePanelProps {
  load?: typeof loadScenarioState;
}

export function ScenarioStatePanel(props: ScenarioStatePanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(SCENARIO_STATE_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadScenarioState, [props.load]);
  const view = usePanelLoad<ScenarioStateResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(SCENARIO_STATE_PANEL_ID, { rowSpan: 2, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={SCENARIO_STATE_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Scenario state">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Scenario state</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<ScenarioStateResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Scenario feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div data-component="WeightedAvg" className="rounded border border-[var(--pellucid-border)] p-2 text-xs">
        <div className="flex items-baseline justify-between">
          <span className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">VOL-WEIGHTED YES</span>
          <span data-field="weighted-avg" className="font-mono">{(r.weightedAvgYes * 100).toFixed(1)}%</span>
        </div>
      </div>
      {r.topRow ? (
        <div data-component="TopRow" className="rounded border border-[var(--pellucid-border)] p-2 text-xs">
          <div className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">HIGHEST YES</div>
          <div data-field="top-question" className="truncate">{r.topRow.question}</div>
          <div className="flex justify-between text-[10px] text-[var(--pellucid-muted)]">
            <span data-field="top-yes">{(r.topRow.yesPrice * 100).toFixed(0)}¢</span>
            <span data-field="top-volume">${r.topRow.volumeUsd.toLocaleString()}</span>
          </div>
        </div>
      ) : null}
      <div className="text-[11px] text-[var(--pellucid-muted)]">
        {r.total} markets · snapshot {formatAssembledAtUtc(r.assembledAtMs)}
        {r.stale ? <> · <span data-field="stale">cached</span></> : null}
      </div>
    </div>
  );
}

registerPanel({
  id: SCENARIO_STATE_PANEL_ID,
  title: "Scenario state",
  blurb: "Volume-weighted aggregate market sentiment.",
  component: ScenarioStatePanel as React.ComponentType<unknown>,
  cacheKeys: ["prediction:scenario-state:current:v1"],
  minTier: 0,
  variants: "*",
});
