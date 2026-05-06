import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadFuelPrices, type FuelPricesResponse } from "../../data/loaders/macro/fuel-prices";
import { registerPanel } from "./index";
import { usePanelLoad } from "./usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "./labelForCode";

export const FUEL_PRICES_PANEL_ID = "macro/fuel-prices";
export const REQUIRED_TIER = 0;

export interface FuelPricesPanelProps {
  load?: typeof loadFuelPrices;
}

export function FuelPricesPanel(props: FuelPricesPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(FUEL_PRICES_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadFuelPrices, [props.load]);
  const view = usePanelLoad<FuelPricesResponse>({ load: () => loader(), requiredTier: REQUIRED_TIER, deps: [loader] });
  useEffect(() => setLayout(FUEL_PRICES_PANEL_ID, { rowSpan: 2, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={FUEL_PRICES_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Fuel prices">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Fuel prices</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<FuelPricesResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading fuel prices…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Fuel pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No fuel rows.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} fuel rows`}>
        {r.rows.map((row, i) => (
          <li
            key={`${row.region}-${row.product}-${i}`}
            data-component="FuelRow"
            data-region={row.region}
            data-tone={row.weekOverWeekChangePct > 0 ? "negative" : row.weekOverWeekChangePct < 0 ? "positive" : "neutral"}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono">{row.region}</span>
            <span className="flex-1 px-2 truncate">{row.product}</span>
            <span data-field="usd-per-gallon" className="font-mono">${row.usdPerGallon.toFixed(2)}</span>
            <span
              data-field="wow"
              className={`ml-2 font-mono ${
                row.weekOverWeekChangePct > 0
                  ? "text-[var(--pellucid-danger)]"
                  : row.weekOverWeekChangePct < 0
                    ? "text-[var(--pellucid-success)]"
                    : "text-[var(--pellucid-muted)]"
              }`}
            >
              {row.weekOverWeekChangePct >= 0 ? "+" : ""}{row.weekOverWeekChangePct.toFixed(1)}%
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
  id: FUEL_PRICES_PANEL_ID,
  title: "Fuel prices",
  blurb: "Regional retail fuel prices + week-over-week deltas.",
  component: FuelPricesPanel as React.ComponentType<unknown>,
  cacheKeys: ["energy:fuel-prices:current:v1"],
  minTier: 0,
  variants: "*",
});
