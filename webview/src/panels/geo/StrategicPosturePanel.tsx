import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadStrategicPosture, type StrategicPostureResponse } from "../../data/loaders/geo/strategic-posture";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const STRATEGIC_POSTURE_PANEL_ID = "geo/strategic-posture";
export const REQUIRED_TIER = 0;

export interface StrategicPosturePanelProps {
  load?: typeof loadStrategicPosture;
}

export function StrategicPosturePanel(props: StrategicPosturePanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(STRATEGIC_POSTURE_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadStrategicPosture, [props.load]);
  const view = usePanelLoad<StrategicPostureResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(STRATEGIC_POSTURE_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={STRATEGIC_POSTURE_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Strategic posture">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Strategic posture</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function readinessClass(r: string): string {
  if (r === "C-1" || r === "DEFCON-1" || r === "DEFCON-2") return "text-[var(--pellucid-danger)]";
  if (r === "C-2" || r === "DEFCON-3") return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<StrategicPostureResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading posture…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Posture feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No theaters reported.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} theaters</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} theater postures`}>
        {r.rows.map((row, i) => (
          <li
            key={`${row.theater}-${i}`}
            data-component="PostureRow"
            data-theater={row.theater}
            data-readiness={row.readiness}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono uppercase">{row.theater}</span>
            <span className="flex-1 px-2 truncate">{row.force}</span>
            <span data-field="readiness" className={`font-mono uppercase ${readinessClass(row.readiness)}`}>{row.readiness}</span>
            <span data-field="headcount" className="ml-2 font-mono">{row.headcount.toLocaleString()}</span>
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
  id: STRATEGIC_POSTURE_PANEL_ID,
  title: "Strategic posture",
  blurb: "Per-theater readiness + force headcount.",
  component: StrategicPosturePanel as React.ComponentType<unknown>,
  cacheKeys: ["military:theater-posture:current:v1"],
  minTier: 0,
  variants: "*",
});
