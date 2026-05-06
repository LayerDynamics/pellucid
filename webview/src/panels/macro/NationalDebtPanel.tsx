import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadNationalDebt, type NationalDebtResponse } from "../../data/loaders/macro/national-debt";
import { EconIndicatorTile, registerPanel } from "./index";
import { usePanelLoad } from "./usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "./labelForCode";

export const NATIONAL_DEBT_PANEL_ID = "macro/national-debt";
export const REQUIRED_TIER = 0;

export interface NationalDebtPanelProps {
  load?: typeof loadNationalDebt;
}

export function NationalDebtPanel(props: NationalDebtPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(NATIONAL_DEBT_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadNationalDebt, [props.load]);
  const view = usePanelLoad<NationalDebtResponse>({ load: () => loader(), requiredTier: REQUIRED_TIER, deps: [loader] });
  useEffect(() => setLayout(NATIONAL_DEBT_PANEL_ID, { rowSpan: 2, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={NATIONAL_DEBT_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="National debt">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">National debt</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<NationalDebtResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading national debt…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Debt pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  return (
    <div data-state="ready" className="flex flex-col gap-3">
      <div className="grid grid-cols-2 gap-2">
        <EconIndicatorTile id="total" code="$B TOTAL" label={r.latest.period} value={`$${r.latest.totalBillionUsd.toFixed(0)}B`} />
        <EconIndicatorTile id="ratio" code="DEBT/GDP" label={r.latest.period} value={`${r.latest.debtToGdpPct.toFixed(1)}%`} />
        <EconIndicatorTile
          id="qoq"
          code="QoQ Δ"
          label="Total"
          value={`${r.qoqDeltaBillionUsd >= 0 ? "+" : ""}${r.qoqDeltaBillionUsd.toFixed(0)}B`}
          tone={r.qoqDeltaBillionUsd >= 0 ? "negative" : "positive"}
        />
        <EconIndicatorTile id="hist" code="HISTORY" label="Quarters" value={String(r.history.length)} />
      </div>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        Snapshot: {formatAssembledAtUtc(r.assembledAtMs)}
        {r.stale ? <> · <span data-field="stale">cached</span></> : null}
      </footer>
    </div>
  );
}

registerPanel({
  id: NATIONAL_DEBT_PANEL_ID,
  title: "National debt",
  blurb: "U.S. total debt + debt-to-GDP from FRED.",
  component: NationalDebtPanel as React.ComponentType<unknown>,
  cacheKeys: ["economic:national-debt:v1"],
  minTier: 0,
  variants: "*",
});
