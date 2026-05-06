import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadFaoFoodPriceIndex, type FaoResponse } from "../../data/loaders/macro/fao-food-price-index";
import { EconIndicatorTile, registerPanel } from "./index";
import { usePanelLoad } from "./usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "./labelForCode";

export const FAO_FOOD_PRICE_INDEX_PANEL_ID = "macro/fao-food-price-index";
export const REQUIRED_TIER = 0;

export interface FaoFoodPriceIndexPanelProps {
  load?: typeof loadFaoFoodPriceIndex;
}

export function FaoFoodPriceIndexPanel(props: FaoFoodPriceIndexPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(FAO_FOOD_PRICE_INDEX_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadFaoFoodPriceIndex, [props.load]);
  const view = usePanelLoad<FaoResponse>({ load: () => loader(), requiredTier: REQUIRED_TIER, deps: [loader] });
  useEffect(() => setLayout(FAO_FOOD_PRICE_INDEX_PANEL_ID, { rowSpan: 2, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={FAO_FOOD_PRICE_INDEX_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="FAO food price index">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">FAO food price index</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<FaoResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading FAO…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>FAO pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  return (
    <div data-state="ready" className="flex flex-col gap-3">
      <EconIndicatorTile
        id="fao-composite"
        code="FFPI"
        label={r.latest.period}
        value={r.latest.composite.toFixed(1)}
        subline={`${r.yoyPct >= 0 ? "+" : ""}${r.yoyPct.toFixed(1)}% YoY`}
        tone={r.yoyPct > 5 ? "negative" : r.yoyPct < -5 ? "positive" : "neutral"}
      />
      <ul className="grid grid-cols-2 gap-1" aria-label={`${r.latest.subindices.length} subindices`}>
        {r.latest.subindices.map(([label, value]) => (
          <li
            key={label}
            data-component="FaoSubindex"
            data-label={label}
            className="flex items-baseline justify-between rounded border border-[var(--pellucid-border)] px-2 py-1 text-[11px]"
          >
            <span className="uppercase font-mono text-[var(--pellucid-muted)]">{label}</span>
            <span className="font-mono">{value.toFixed(1)}</span>
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
  id: FAO_FOOD_PRICE_INDEX_PANEL_ID,
  title: "FAO food price index",
  blurb: "Composite + sub-index food prices from FAO.",
  component: FaoFoodPriceIndexPanel as React.ComponentType<unknown>,
  cacheKeys: ["economic:fao-food-price-index:v1"],
  minTier: 0,
  variants: "*",
});
