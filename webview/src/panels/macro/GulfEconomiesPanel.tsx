import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadGulfEconomies, type GulfResponse } from "../../data/loaders/macro/gulf-economies";
import { CountryEconCard, registerPanel, type CountryEconSummary } from "./index";
import { usePanelLoad } from "./usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "./labelForCode";

export const GULF_ECONOMIES_PANEL_ID = "macro/gulf-economies";
export const REQUIRED_TIER = 0;

export interface GulfEconomiesPanelProps {
  load?: typeof loadGulfEconomies;
}

export function GulfEconomiesPanel(props: GulfEconomiesPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(GULF_ECONOMIES_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadGulfEconomies, [props.load]);
  const view = usePanelLoad<GulfResponse>({ load: () => loader(), requiredTier: REQUIRED_TIER, deps: [loader] });
  useEffect(() => setLayout(GULF_ECONOMIES_PANEL_ID, { rowSpan: 2, colSpan: 4 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={GULF_ECONOMIES_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Gulf economies">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Gulf economies</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<GulfResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading Gulf economies…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Gulf pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No country data.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-3">
      <ul className="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3" aria-label={`${r.rows.length} countries`}>
        {r.rows.map((row) => {
          const summary: CountryEconSummary = {
            country: row.country,
            iso: row.iso,
            gdpUsdBillion: row.gdpUsdBillion,
            gdpYoyPct: row.gdpYoyPct,
            inflationYoyPct: row.inflationYoyPct,
            unemploymentPct: row.unemploymentPct,
            policyRatePct: row.policyRatePct,
          };
          return (
            <li key={row.iso}>
              <CountryEconCard summary={summary} />
            </li>
          );
        })}
      </ul>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        Snapshot: {formatAssembledAtUtc(r.assembledAtMs)}
        {r.stale ? <> · <span data-field="stale">cached</span></> : null}
      </footer>
    </div>
  );
}

registerPanel({
  id: GULF_ECONOMIES_PANEL_ID,
  title: "Gulf economies",
  blurb: "GCC + Iran economic dashboard.",
  component: GulfEconomiesPanel as React.ComponentType<unknown>,
  cacheKeys: ["economic:gulf-economies:v1"],
  minTier: 0,
  variants: "*",
});
