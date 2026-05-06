import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadFinancialStress, type FsiResponse } from "../../data/loaders/macro/financial-stress";
import { EconIndicatorTile, registerPanel } from "./index";
import { usePanelLoad } from "./usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "./labelForCode";

export const FSI_PANEL_ID = "macro/fsi";
export const REQUIRED_TIER = 0;

export interface FSIPanelProps {
  load?: typeof loadFinancialStress;
}

export function FSIPanel(props: FSIPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(FSI_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadFinancialStress, [props.load]);
  const view = usePanelLoad<FsiResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(FSI_PANEL_ID, { rowSpan: 2, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section
      data-panel-id={FSI_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Financial stress index"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Financial stress</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<FsiResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading FSI…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>FSI pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  const delta = r.prior ? r.latest.value - r.prior.value : 0;
  const tone = delta > 0 ? "negative" : delta < 0 ? "positive" : "neutral";
  return (
    <div data-state="ready" className="flex flex-col gap-3">
      <EconIndicatorTile
        id="fsi"
        code={r.seriesCode}
        label="Latest"
        value={r.latest.value.toFixed(2)}
        subline={r.prior ? `Δ ${delta >= 0 ? "+" : ""}${delta.toFixed(2)}` : r.latest.date}
        tone={tone as "positive" | "negative" | "neutral"}
      />
      <FsiSparkline data={r.history} />
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        {r.latest.date} · snapshot {formatAssembledAtUtc(r.assembledAtMs)}
        {r.stale ? <> · <span data-field="stale">cached</span></> : null}
      </footer>
    </div>
  );
}

const W = 320;
const H = 56;

function FsiSparkline({ data }: { data: { date: string; value: number }[] }): ReactElement {
  if (data.length < 2) {
    return <div data-component="FsiSparkline" data-state="too-few" className="text-[11px] text-[var(--pellucid-muted)]">Not enough history.</div>;
  }
  const ys = data.map((d) => d.value);
  const lo = Math.min(...ys);
  const hi = Math.max(...ys);
  const range = Math.max(0.001, hi - lo);
  const stepX = data.length > 1 ? W / (data.length - 1) : W;
  const path = data
    .map((d, i) => {
      const x = i * stepX;
      const y = H - ((d.value - lo) / range) * H;
      return `${i === 0 ? "M" : "L"} ${x.toFixed(1)} ${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg
      data-component="FsiSparkline"
      width={W}
      height={H}
      viewBox={`0 0 ${W} ${H}`}
      role="img"
      aria-label={`${data.length}-week financial stress sparkline`}
    >
      <path data-field="curve-path" d={path} fill="none" stroke="currentColor" strokeWidth={1.5} />
    </svg>
  );
}

registerPanel({
  id: FSI_PANEL_ID,
  title: "Financial stress",
  blurb: "St. Louis Fed Financial Stress Index trend.",
  component: FSIPanel as React.ComponentType<unknown>,
  cacheKeys: ["economic:financial-stress:v1"],
  minTier: 0,
  variants: "*",
});
