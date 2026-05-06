import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadRenewableMix, type RenewableResponse } from "../../data/loaders/energy/renewable";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const RENEWABLE_ENERGY_PANEL_ID = "energy/renewable";
export const REQUIRED_TIER = 0;

export interface RenewableEnergyPanelProps {
  load?: typeof loadRenewableMix;
}

export function RenewableEnergyPanel(props: RenewableEnergyPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(RENEWABLE_ENERGY_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadRenewableMix, [props.load]);
  const view = usePanelLoad<RenewableResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(RENEWABLE_ENERGY_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section
      data-panel-id={RENEWABLE_ENERGY_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Renewable energy mix"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Renewable energy mix</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<RenewableResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading renewable mix…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Renewable pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No source data.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">
        Total capacity: <span data-field="total-capacity-gw" className="font-mono">{r.totalCapacityGw.toFixed(0)} GW</span>
        <span className="ml-2 text-[var(--pellucid-muted)]">{r.period}</span>
      </div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} renewable sources`}>
        {r.rows.map((row) => (
          <li
            key={row.source}
            data-component="RenewableRow"
            data-source={row.source}
            className="flex flex-col gap-0.5 text-[11px]"
          >
            <div className="flex items-baseline justify-between">
              <span className="font-mono uppercase">{row.source}</span>
              <span data-field="capacity-gw" className="font-mono">{row.capacityGw.toFixed(0)} GW</span>
              <span
                data-field="yoy"
                className={`ml-2 font-mono ${row.yoyPct >= 0 ? "text-[var(--pellucid-success)]" : "text-[var(--pellucid-danger)]"}`}
              >
                {row.yoyPct >= 0 ? "+" : ""}{row.yoyPct.toFixed(1)}%
              </span>
            </div>
            <div
              data-field="share-bar"
              className="h-1 rounded bg-[var(--pellucid-border)]"
              aria-hidden="true"
            >
              <div
                className="h-full rounded bg-[var(--pellucid-success)]"
                style={{ width: `${Math.min(100, row.sharePct).toFixed(1)}%` }}
              />
            </div>
            <span data-field="share-pct" className="text-[10px] text-[var(--pellucid-muted)]">
              {row.sharePct.toFixed(1)}% of total
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
  id: RENEWABLE_ENERGY_PANEL_ID,
  title: "Renewable energy mix",
  blurb: "Per-source capacity in GW + YoY %.",
  component: RenewableEnergyPanel as React.ComponentType<unknown>,
  cacheKeys: ["energy:renewable-mix:current:v1"],
  minTier: 0,
  variants: "*",
});
