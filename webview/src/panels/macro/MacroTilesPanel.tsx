import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadMacroTiles, type MacroTilesResponse } from "../../data/loaders/macro/macro-tiles";
import { EconIndicatorTile, registerPanel, type IndicatorTone } from "./index";
import { usePanelLoad } from "./usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "./labelForCode";

export const MACRO_TILES_PANEL_ID = "macro/tiles";
export const REQUIRED_TIER = 0;

export interface MacroTilesPanelProps {
  load?: typeof loadMacroTiles;
}

export function MacroTilesPanel(props: MacroTilesPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(MACRO_TILES_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadMacroTiles, [props.load]);
  const view = usePanelLoad<MacroTilesResponse>({ load: () => loader(), requiredTier: REQUIRED_TIER, deps: [loader] });
  useEffect(() => setLayout(MACRO_TILES_PANEL_ID, { rowSpan: 1, colSpan: 4 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={MACRO_TILES_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Macro tiles">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Macro tiles</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function asTone(s: string): IndicatorTone {
  return s === "positive" || s === "negative" ? s : "neutral";
}

function renderBody(view: ReturnType<typeof usePanelLoad<MacroTilesResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading tiles…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Tiles pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <ul className="grid grid-cols-2 gap-2 md:grid-cols-4" aria-label={`${out.response.tiles.length} macro tiles`}>
        {out.response.tiles.map((t) => (
          <li key={t.code}>
            <EconIndicatorTile
              id={t.code}
              code={t.code}
              label={t.label}
              value={t.value}
              {...(t.subline ? { subline: t.subline } : {})}
              tone={asTone(t.tone)}
            />
          </li>
        ))}
      </ul>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        Snapshot: {formatAssembledAtUtc(out.response.assembledAtMs)}
        {out.response.stale ? <> · <span data-field="stale">cached</span></> : null}
      </footer>
    </div>
  );
}

registerPanel({
  id: MACRO_TILES_PANEL_ID,
  title: "Macro tiles",
  blurb: "Quick-glance macro tile grid.",
  component: MacroTilesPanel as React.ComponentType<unknown>,
  cacheKeys: ["economic:fred-latest:UNRATE:v1", "economic:fred-latest:CPIAUCSL:v1", "economic:financial-stress:v1", "energy:fuel-prices:current:v1"],
  minTier: 0,
  variants: "*",
});
