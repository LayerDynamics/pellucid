import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadOilInventories, type OilInventoriesResponse } from "../../data/loaders/energy/oil-inventories";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const OIL_INVENTORIES_PANEL_ID = "energy/oil-inventories";
export const REQUIRED_TIER = 0;

export interface OilInventoriesPanelProps {
  load?: typeof loadOilInventories;
}

export function OilInventoriesPanel(props: OilInventoriesPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(OIL_INVENTORIES_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadOilInventories, [props.load]);
  const view = usePanelLoad<OilInventoriesResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(OIL_INVENTORIES_PANEL_ID, { rowSpan: 2, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section
      data-panel-id={OIL_INVENTORIES_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Oil inventories"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Oil inventories</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<OilInventoriesResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading oil stocks…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Inventory pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No products reported.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} oil-stock rows`}>
        {r.rows.map((row, i) => (
          <li
            key={`${row.product}-${i}`}
            data-component="OilStocksRow"
            data-product={row.product}
            data-tone={row.wowDeltaMb < 0 ? "negative" : row.wowDeltaMb > 0 ? "positive" : "neutral"}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono uppercase">{row.product}</span>
            <span className="flex-1 px-2 truncate text-[var(--pellucid-muted)]">{row.period}</span>
            <span data-field="value-mb" className="font-mono">{row.valueMb.toFixed(1)} mb</span>
            <span
              data-field="wow"
              className={`ml-2 font-mono ${row.wowDeltaMb < 0 ? "text-[var(--pellucid-danger)]" : row.wowDeltaMb > 0 ? "text-[var(--pellucid-success)]" : "text-[var(--pellucid-muted)]"}`}
            >
              {row.wowDeltaMb >= 0 ? "+" : ""}{row.wowDeltaMb.toFixed(2)}
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
  id: OIL_INVENTORIES_PANEL_ID,
  title: "Oil inventories",
  blurb: "EIA weekly petroleum-stocks snapshot.",
  component: OilInventoriesPanel as React.ComponentType<unknown>,
  cacheKeys: ["eia:petroleum-stocks:latest:v1"],
  minTier: 0,
  variants: "*",
});
