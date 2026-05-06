import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadEnergyComplex, type ComplexResponse } from "../../data/loaders/energy/complex";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const ENERGY_COMPLEX_PANEL_ID = "energy/complex";
export const REQUIRED_TIER = 0;

export interface EnergyComplexPanelProps {
  load?: typeof loadEnergyComplex;
}

export function EnergyComplexPanel(props: EnergyComplexPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(ENERGY_COMPLEX_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadEnergyComplex, [props.load]);
  const view = usePanelLoad<ComplexResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(ENERGY_COMPLEX_PANEL_ID, { rowSpan: 1, colSpan: 4 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section
      data-panel-id={ENERGY_COMPLEX_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Energy complex"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Energy complex</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<ComplexResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading energy complex…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Energy pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <ul className="grid grid-cols-1 md:grid-cols-3 gap-2" aria-label={`${r.tiles.length} energy-complex tiles`}>
        {r.tiles.map((t) => (
          <li
            key={t.code}
            data-component="EnergyComplexTile"
            data-code={t.code}
            data-tone={t.tone}
            className={`rounded border p-2 text-xs ${
              t.tone === "negative"
                ? "border-[var(--pellucid-danger)]"
                : t.tone === "positive"
                  ? "border-[var(--pellucid-success)]"
                  : "border-[var(--pellucid-border)]"
            }`}
          >
            <div className="flex items-baseline justify-between">
              <span className="font-mono uppercase text-[10px] text-[var(--pellucid-muted)]">{t.code}</span>
              <span data-field="value" className="font-mono">{t.value}</span>
            </div>
            <div data-field="label" className="text-[var(--pellucid-fg)]">{t.label}</div>
            {t.subline ? <div data-field="subline" className="text-[10px] text-[var(--pellucid-muted)]">{t.subline}</div> : null}
          </li>
        ))}
      </ul>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        {r.availableTiles} of 3 signals · snapshot {formatAssembledAtUtc(r.assembledAtMs)}
        {r.stale ? <> · <span data-field="stale">cached</span></> : null}
      </footer>
    </div>
  );
}

registerPanel({
  id: ENERGY_COMPLEX_PANEL_ID,
  title: "Energy complex",
  blurb: "Fuel + EU gas storage + SPR composite.",
  component: EnergyComplexPanel as React.ComponentType<unknown>,
  cacheKeys: ["energy:fuel-prices:current:v1", "energy:gie-gas-storage:current:v1", "energy:spr-status:current:v1"],
  minTier: 0,
  variants: "*",
});
