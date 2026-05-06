import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadScenarioLibrary, type ScenarioLibraryResponse } from "../../data/loaders/forecast/scenario-library";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const SCENARIO_LIBRARY_PANEL_ID = "forecast/scenario-library";
export const REQUIRED_TIER = 0;

export interface ScenarioLibraryPanelProps {
  load?: typeof loadScenarioLibrary;
}

export function ScenarioLibraryPanel(props: ScenarioLibraryPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(SCENARIO_LIBRARY_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadScenarioLibrary, [props.load]);
  const view = usePanelLoad<ScenarioLibraryResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(SCENARIO_LIBRARY_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={SCENARIO_LIBRARY_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Scenario library">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Scenario library</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function probClass(p: number): string {
  if (p >= 0.7) return "text-[var(--pellucid-danger)]";
  if (p >= 0.4) return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<ScenarioLibraryResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading library…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Library feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No scenarios.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} scenarios</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} scenarios`}>
        {r.rows.map((row) => (
          <li
            key={row.id}
            data-component="ScenarioRow"
            data-id={row.id}
            data-domain={row.domain}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">{row.domain}</span>
            <span className="flex-1 px-2 truncate">{row.title}</span>
            <span data-field="probability" className={`font-mono ${probClass(row.probability)}`}>{(row.probability * 100).toFixed(0)}%</span>
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
  id: SCENARIO_LIBRARY_PANEL_ID,
  title: "Scenario library",
  blurb: "Catalogued scenarios sorted by probability.",
  component: ScenarioLibraryPanel as React.ComponentType<unknown>,
  cacheKeys: ["prediction:scenario-library:v1"],
  minTier: 0,
  variants: "*",
});
