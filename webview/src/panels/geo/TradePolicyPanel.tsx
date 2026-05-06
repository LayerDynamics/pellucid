import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadTradePolicy, type TradePolicyResponse } from "../../data/loaders/geo/trade-policy";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const TRADE_POLICY_PANEL_ID = "geo/trade-policy";
export const REQUIRED_TIER = 0;

export interface TradePolicyPanelProps {
  load?: typeof loadTradePolicy;
}

export function TradePolicyPanel(props: TradePolicyPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(TRADE_POLICY_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadTradePolicy, [props.load]);
  const view = usePanelLoad<TradePolicyResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(TRADE_POLICY_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={TRADE_POLICY_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Trade policy">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Trade policy alerts</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<TradePolicyResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading tariffs…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Tariff feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No tariff alerts.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">
        {r.total} alerts · net rate Δ{" "}
        <span
          data-field="total-rate-delta-pp"
          className={`font-mono ${r.totalRateDeltaPp >= 0 ? "text-[var(--pellucid-warn)]" : "text-[var(--pellucid-success)]"}`}
        >
          {r.totalRateDeltaPp >= 0 ? "+" : ""}{r.totalRateDeltaPp.toFixed(1)}pp
        </span>
      </div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} tariff alerts`}>
        {r.rows.map((row, i) => (
          <li
            key={`${row.authority}-${row.hsCode}-${i}`}
            data-component="TariffRow"
            data-authority={row.authority}
            data-hs-code={row.hsCode}
            className="flex flex-col gap-0.5 text-[11px] rounded border border-[var(--pellucid-border)] p-1.5"
          >
            <div className="flex items-baseline justify-between">
              <span data-field="authority" className="font-mono uppercase">{row.authority}</span>
              <span data-field="route" className="font-mono text-[var(--pellucid-muted)]">{row.origin}→{row.destination}</span>
              <span
                data-field="rate-delta-pp"
                className={`font-mono ${row.rateDeltaPp >= 0 ? "text-[var(--pellucid-warn)]" : "text-[var(--pellucid-success)]"}`}
              >
                {row.rateDeltaPp >= 0 ? "+" : ""}{row.rateDeltaPp.toFixed(1)}pp
              </span>
            </div>
            <div data-field="headline" className="text-[var(--pellucid-fg)]">{row.headline}</div>
            <div className="flex justify-between text-[10px] text-[var(--pellucid-muted)]">
              <span data-field="hs-code">HS {row.hsCode} · {row.product}</span>
              <span data-field="effective">{row.effective}</span>
            </div>
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
  id: TRADE_POLICY_PANEL_ID,
  title: "Trade policy",
  blurb: "USTR / WTO / EU tariff alerts + rate deltas.",
  component: TradePolicyPanel as React.ComponentType<unknown>,
  cacheKeys: ["trade:tariff-alerts:current:v1"],
  minTier: 0,
  variants: "*",
});
