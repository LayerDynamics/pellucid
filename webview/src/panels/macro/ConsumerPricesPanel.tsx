import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadCpiList, type CpiListResponse } from "../../data/loaders/macro/consumer-prices-list";
import { CpiBreakdown, registerPanel } from "./index";
import { usePanelLoad } from "./usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "./labelForCode";

export const CONSUMER_PRICES_PANEL_ID = "macro/consumer-prices";
export const REQUIRED_TIER = 0;

export interface ConsumerPricesPanelProps {
  load?: typeof loadCpiList;
}

export function ConsumerPricesPanel(props: ConsumerPricesPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(CONSUMER_PRICES_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadCpiList, [props.load]);
  const view = usePanelLoad<CpiListResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => {
    setLayout(CONSUMER_PRICES_PANEL_ID, { rowSpan: 2, colSpan: 3 });
  }, [setLayout]);
  if (isHidden) return <></>;
  return (
    <section
      data-panel-id={CONSUMER_PRICES_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Consumer prices"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Consumer prices</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<CpiListResponse>>): ReactElement {
  if (view.kind === "loading") {
    return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading CPI…</div>;
  }
  if (view.kind === "locked") {
    return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  }
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") {
      return (
        <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]">
          <strong>CPI pipeline offline.</strong>
          {out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}
        </div>
      );
    }
    return (
      <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]">
        <strong>{labelForCode(out.code)}</strong><br />{out.message}
      </div>
    );
  }
  return (
    <div data-state="ready" className="flex flex-col gap-3">
      {out.response.rows.map((row) => (
        <article
          key={row.region}
          data-component="CpiRegionRow"
          data-region={row.region}
          className="rounded border border-[var(--pellucid-border)] p-2"
        >
          <header className="flex items-baseline justify-between text-[11px]">
            <span className="font-mono uppercase">{row.region}</span>
            <span className="text-[var(--pellucid-muted)]">{row.period}</span>
          </header>
          <CpiBreakdown
            components={(row.components ?? []).map((c) => ({ label: c.label, yoyPct: c.yoyPct, ...(typeof c.weight === "number" ? { weight: c.weight } : {}) }))}
            headline={`${row.yoyPct >= 0 ? "+" : ""}${row.yoyPct.toFixed(1)}% YoY · ${row.momPct >= 0 ? "+" : ""}${row.momPct.toFixed(1)}% MoM`}
          />
        </article>
      ))}
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        Snapshot: {formatAssembledAtUtc(out.response.assembledAtMs)}
        {out.response.stale ? <> · <span data-field="stale">cached</span></> : null}
      </footer>
    </div>
  );
}

registerPanel({
  id: CONSUMER_PRICES_PANEL_ID,
  title: "Consumer prices",
  blurb: "Latest US + EU CPI with component breakdowns.",
  component: ConsumerPricesPanel as React.ComponentType<unknown>,
  cacheKeys: ["consumer-prices:latest-cpi:US:v1", "consumer-prices:latest-cpi:EU:v1"],
  minTier: 0,
  variants: "*",
});
