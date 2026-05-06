import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadVolcanoActivity, type VolcanoResponse } from "../../data/loaders/climate/volcano";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const VOLCANO_PANEL_ID = "climate/volcano-activity";
export const REQUIRED_TIER = 0;

export interface VolcanoActivityPanelProps {
  load?: typeof loadVolcanoActivity;
}

export function VolcanoActivityPanel(props: VolcanoActivityPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(VOLCANO_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadVolcanoActivity, [props.load]);
  const view = usePanelLoad<VolcanoResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(VOLCANO_PANEL_ID, { rowSpan: 2, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={VOLCANO_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Volcano activity">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Volcano activity</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<VolcanoResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading volcanoes…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>EONET pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No active volcanoes.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} active</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} volcanoes`}>
        {r.rows.map((row) => (
          <li
            key={row.id}
            data-component="VolcanoRow"
            data-id={row.id}
            data-status={row.status}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono">{row.name}</span>
            <span className="text-[var(--pellucid-muted)]">{row.country}</span>
            <span data-field="status" className="ml-2 font-mono uppercase text-[var(--pellucid-warn)]">{row.status}</span>
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
  id: VOLCANO_PANEL_ID,
  title: "Volcano activity",
  blurb: "Active volcano feed (NASA EONET).",
  component: VolcanoActivityPanel as React.ComponentType<unknown>,
  cacheKeys: ["natural:volcano-feed:current:v1"],
  minTier: 0,
  variants: "*",
});
