import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadNowCast, type NowCastResponse } from "../../data/loaders/forecast/now-cast";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const NOW_CAST_PANEL_ID = "forecast/now-cast";
export const REQUIRED_TIER = 0;

export interface NowCastPanelProps {
  load?: typeof loadNowCast;
}

export function NowCastPanel(props: NowCastPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(NOW_CAST_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadNowCast, [props.load]);
  const view = usePanelLoad<NowCastResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(NOW_CAST_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={NOW_CAST_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Metaculus now-cast">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Now-cast (Metaculus)</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function trendClass(trend: string): string {
  if (trend === "rising") return "text-[var(--pellucid-warn)]";
  if (trend === "falling") return "text-[var(--pellucid-success)]";
  return "text-[var(--pellucid-muted)]";
}

function trendArrow(t: string): string {
  if (t === "rising") return "▲";
  if (t === "falling") return "▼";
  return "◆";
}

function renderBody(view: ReturnType<typeof usePanelLoad<NowCastResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading now-cast…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Now-cast feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No now-cast questions.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} questions</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} now-casts`}>
        {r.rows.map((row) => (
          <li
            key={row.id}
            data-component="NowCastRow"
            data-id={row.id}
            data-trend={row.trend}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="flex-1 truncate">{row.question}</span>
            <span data-field="probability" className="ml-2 font-mono">{(row.probability * 100).toFixed(0)}%</span>
            <span data-field="trend" className={`ml-2 font-mono ${trendClass(row.trend)}`}>{trendArrow(row.trend)} {row.trend}</span>
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
  id: NOW_CAST_PANEL_ID,
  title: "Now-cast (Metaculus)",
  blurb: "Crowd now-cast probabilities + trend direction.",
  component: NowCastPanel as React.ComponentType<unknown>,
  cacheKeys: ["forecast:now-cast:summary:v1"],
  minTier: 0,
  variants: "*",
});
