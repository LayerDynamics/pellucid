import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadEnergyCrisis, type CrisisResponse } from "../../data/loaders/energy/crisis";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const ENERGY_CRISIS_PANEL_ID = "energy/crisis";
export const REQUIRED_TIER = 0;

export interface EnergyCrisisPanelProps {
  load?: typeof loadEnergyCrisis;
}

export function EnergyCrisisPanel(props: EnergyCrisisPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(ENERGY_CRISIS_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadEnergyCrisis, [props.load]);
  const view = usePanelLoad<CrisisResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(ENERGY_CRISIS_PANEL_ID, { rowSpan: 1, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section
      data-panel-id={ENERGY_CRISIS_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Energy crisis risk"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Energy crisis risk</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<CrisisResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading crisis score…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Crisis pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  const cls =
    r.level === "high"
      ? "text-[var(--pellucid-danger)] border-[var(--pellucid-danger)]"
      : r.level === "elevated"
        ? "text-[var(--pellucid-warn)] border-[var(--pellucid-warn)]"
        : "text-[var(--pellucid-success)] border-[var(--pellucid-success)]";
  return (
    <div data-state="ready" data-level={r.level} className="flex flex-col gap-2">
      <div data-component="CrisisGauge" className={`flex flex-col rounded border p-2 ${cls}`}>
        <div className="flex items-baseline justify-between">
          <span data-field="level" className="text-xs uppercase font-mono">{r.level}</span>
          <span data-field="score" className="font-mono">{r.score}/100</span>
        </div>
        <div data-field="headline" className="text-sm font-semibold">{r.headline}</div>
        <div data-field="rationale" className="text-[11px] text-[var(--pellucid-muted)]">{r.rationale}</div>
      </div>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        Snapshot: {formatAssembledAtUtc(r.assembledAtMs)}
        {r.stale ? <> · <span data-field="stale">cached</span></> : null}
      </footer>
    </div>
  );
}

registerPanel({
  id: ENERGY_CRISIS_PANEL_ID,
  title: "Energy crisis risk",
  blurb: "Composite gas-storage + SPR risk score.",
  component: EnergyCrisisPanel as React.ComponentType<unknown>,
  cacheKeys: ["energy:gie-gas-storage:current:v1", "energy:spr-status:current:v1"],
  minTier: 0,
  variants: "*",
});
