import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadWildfire, type WildfireResponse } from "../../data/loaders/climate/wildfire";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const WILDFIRE_PANEL_ID = "climate/wildfire";
export const REQUIRED_TIER = 0;

export interface WildfirePanelProps {
  load?: typeof loadWildfire;
}

export function WildfirePanel(props: WildfirePanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(WILDFIRE_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadWildfire, [props.load]);
  const view = usePanelLoad<WildfireResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(WILDFIRE_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={WILDFIRE_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Active wildfires">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Active wildfires</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<WildfireResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading wildfires…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Wildfire pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No active fires.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} active fires</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} fires`}>
        {r.rows.map((row) => (
          <li
            key={`${row.label}-${row.lat}-${row.lon}`}
            data-component="WildfireRow"
            data-tone={row.containmentPct >= 100 ? "neutral" : "negative"}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono">{row.label}</span>
            <span className="text-[var(--pellucid-muted)]">{row.region}</span>
            <span data-field="acres" className="ml-2 font-mono">{row.acresBurned.toFixed(0)} ac</span>
            <span
              data-field="containment"
              className={`ml-2 font-mono ${row.containmentPct >= 100 ? "text-[var(--pellucid-success)]" : "text-[var(--pellucid-warn)]"}`}
            >
              {row.containmentPct.toFixed(0)}%
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
  id: WILDFIRE_PANEL_ID,
  title: "Active wildfires",
  blurb: "FIRMS active perimeters with containment %.",
  component: WildfirePanel as React.ComponentType<unknown>,
  cacheKeys: ["wildfire:active-perimeters:current:v1"],
  minTier: 0,
  variants: "*",
});
