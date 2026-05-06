import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadHormuz, type HormuzResponse } from "../../data/loaders/energy/hormuz";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const HORMUZ_PANEL_ID = "energy/hormuz";
export const REQUIRED_TIER = 0;

export interface HormuzPanelProps {
  load?: typeof loadHormuz;
}

export function HormuzPanel(props: HormuzPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(HORMUZ_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadHormuz, [props.load]);
  const view = usePanelLoad<HormuzResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(HORMUZ_PANEL_ID, { rowSpan: 2, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section
      data-panel-id={HORMUZ_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Strait of Hormuz transits"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Strait of Hormuz transits</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<HormuzResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading Hormuz…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Hormuz pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No transit rows.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">
        Total throughput: <span data-field="total-mb-per-day" className="font-mono">{r.totalMbPerDay.toFixed(1)} mb/day</span>
        <span className="ml-2 text-[var(--pellucid-muted)]">{r.period}</span>
      </div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} chokepoint rows`}>
        {r.rows.map((row) => (
          <li
            key={row.product}
            data-component="HormuzRow"
            data-product={row.product}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono uppercase">{row.product}</span>
            <span data-field="mb-per-day" className="font-mono">{row.mbPerDay.toFixed(1)} mb/day</span>
            <span data-field="share" className="ml-2 font-mono text-[var(--pellucid-muted)]">{row.sharePct.toFixed(1)}%</span>
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
  id: HORMUZ_PANEL_ID,
  title: "Strait of Hormuz transits",
  blurb: "Daily mb-equivalent flow through Hormuz.",
  component: HormuzPanel as React.ComponentType<unknown>,
  cacheKeys: ["energy:hormuz-transits:current:v1"],
  minTier: 0,
  variants: "*",
});
