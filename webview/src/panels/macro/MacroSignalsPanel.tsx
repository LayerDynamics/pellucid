import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import {
  loadMacroSignals,
  type MacroSignalsResponse,
  type SignalDirection,
} from "../../data/loaders/macro/macro-signals";
import { registerPanel } from "./index";
import { usePanelLoad } from "./usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "./labelForCode";

export const MACRO_SIGNALS_PANEL_ID = "macro/signals";
export const REQUIRED_TIER = 0;

export interface MacroSignalsPanelProps {
  load?: typeof loadMacroSignals;
}

export function MacroSignalsPanel(props: MacroSignalsPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(MACRO_SIGNALS_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadMacroSignals, [props.load]);
  const view = usePanelLoad<MacroSignalsResponse>({ load: () => loader(), requiredTier: REQUIRED_TIER, deps: [loader] });
  useEffect(() => setLayout(MACRO_SIGNALS_PANEL_ID, { rowSpan: 2, colSpan: 2 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={MACRO_SIGNALS_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Macro signals">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Macro signals</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<MacroSignalsResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading signals…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Signals pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <ul className="flex flex-col gap-2" aria-label={`${out.response.signals.length} macro signals`}>
        {out.response.signals.map((s) => (
          <li
            key={s.code}
            data-component="MacroSignalRow"
            data-code={s.code}
            data-direction={s.direction}
            className="flex flex-col rounded border border-[var(--pellucid-border)] p-2"
          >
            <div className="flex items-baseline justify-between text-xs">
              <span data-field="headline" className={`font-semibold ${directionClass(s.direction)}`}>
                {arrow(s.direction)} {s.headline}
              </span>
              <span className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">{s.code}</span>
            </div>
            <span data-field="rationale" className="text-[11px] text-[var(--pellucid-muted)]">{s.rationale}</span>
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

export function arrow(d: SignalDirection): string {
  return d === "rising" ? "▲" : d === "falling" ? "▼" : "◆";
}

export function directionClass(d: SignalDirection): string {
  if (d === "rising") return "text-[var(--pellucid-warn)]";
  if (d === "falling") return "text-[var(--pellucid-success)]";
  return "text-[var(--pellucid-muted)]";
}

registerPanel({
  id: MACRO_SIGNALS_PANEL_ID,
  title: "Macro signals",
  blurb: "Direction-tagged macro alerts (UNRATE / CPI / FSI).",
  component: MacroSignalsPanel as React.ComponentType<unknown>,
  cacheKeys: ["economic:fred-latest:UNRATE:v1", "economic:fred-latest:CPIAUCSL:v1", "economic:financial-stress:v1"],
  minTier: 0,
  variants: "*",
});
