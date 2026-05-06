import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadGroceryBasket, type GroceryResponse } from "../../data/loaders/macro/grocery-basket";
import { registerPanel } from "./index";
import { usePanelLoad } from "./usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "./labelForCode";

export const GROCERY_BASKET_PANEL_ID = "macro/grocery-basket";
export const REQUIRED_TIER = 0;

export interface GroceryBasketPanelProps {
  load?: typeof loadGroceryBasket;
}

export function GroceryBasketPanel(props: GroceryBasketPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(GROCERY_BASKET_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadGroceryBasket, [props.load]);
  const view = usePanelLoad<GroceryResponse>({ load: () => loader(), requiredTier: REQUIRED_TIER, deps: [loader] });
  useEffect(() => setLayout(GROCERY_BASKET_PANEL_ID, { rowSpan: 2, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={GROCERY_BASKET_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Grocery basket">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Grocery basket</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<GroceryResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading basket…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Basket pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No countries.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} grocery baskets`}>
        {r.rows.map((row) => (
          <li
            key={row.iso}
            data-component="GroceryRow"
            data-iso={row.iso}
            data-tone={row.basketYoyPct > 4 ? "negative" : "neutral"}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono uppercase">{row.iso}</span>
            <span className="flex-1 px-2 truncate">{row.country}</span>
            <span data-field="basket-usd" className="font-mono">${row.basketUsd.toFixed(0)}</span>
            <span
              data-field="basket-yoy"
              className={`ml-2 font-mono ${row.basketYoyPct > 4 ? "text-[var(--pellucid-danger)]" : "text-[var(--pellucid-muted)]"}`}
            >
              {row.basketYoyPct >= 0 ? "+" : ""}{row.basketYoyPct.toFixed(1)}%
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
  id: GROCERY_BASKET_PANEL_ID,
  title: "Grocery basket",
  blurb: "Per-country grocery basket cost in USD + YoY %.",
  component: GroceryBasketPanel as React.ComponentType<unknown>,
  cacheKeys: ["consumer-prices:grocery-basket:v1"],
  minTier: 0,
  variants: "*",
});
