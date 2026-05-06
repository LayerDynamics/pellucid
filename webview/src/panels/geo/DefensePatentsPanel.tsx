import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadDefensePatents, type DefensePatentsResponse } from "../../data/loaders/geo/defense-patents";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const DEFENSE_PATENTS_PANEL_ID = "geo/defense-patents";
export const REQUIRED_TIER = 0;

export interface DefensePatentsPanelProps {
  load?: typeof loadDefensePatents;
}

export function DefensePatentsPanel(props: DefensePatentsPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(DEFENSE_PATENTS_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadDefensePatents, [props.load]);
  const view = usePanelLoad<DefensePatentsResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(DEFENSE_PATENTS_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={DEFENSE_PATENTS_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Defense patents">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Defense patents</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<DefensePatentsResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading patents…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>USPTO feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No patent activity.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">
        {r.total} CPC classes · <span data-field="total-filings-30d" className="font-mono">{r.totalFilings30d}</span> filings (30 d) · {r.period}
      </div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} CPC classes`}>
        {r.rows.map((row) => (
          <li
            key={row.cpcClass}
            data-component="PatentClassRow"
            data-cpc={row.cpcClass}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span data-field="cpc-class" className="font-mono uppercase">{row.cpcClass}</span>
            <span className="flex-1 px-2 truncate">{row.label}</span>
            <span data-field="filings-30d" className="font-mono">{row.filings30d}</span>
            <span
              data-field="yoy-pct"
              className={`ml-2 font-mono ${row.yoyPct >= 0 ? "text-[var(--pellucid-warn)]" : "text-[var(--pellucid-success)]"}`}
            >
              {row.yoyPct >= 0 ? "+" : ""}{row.yoyPct.toFixed(1)}%
            </span>
            <span data-field="top-filer" className="ml-2 text-[10px] text-[var(--pellucid-muted)]">{row.topFiler}</span>
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
  id: DEFENSE_PATENTS_PANEL_ID,
  title: "Defense patents",
  blurb: "USPTO defense-related CPC class filings + YoY %.",
  component: DefensePatentsPanel as React.ComponentType<unknown>,
  cacheKeys: ["defense:patent-trends:v1"],
  minTier: 0,
  variants: "*",
});
