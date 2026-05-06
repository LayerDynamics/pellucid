import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadSanctionsPressure, type SanctionsPressureResponse } from "../../data/loaders/geo/sanctions-pressure";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const SANCTIONS_PRESSURE_PANEL_ID = "geo/sanctions-pressure";
export const REQUIRED_TIER = 0;

export interface SanctionsPressurePanelProps {
  load?: typeof loadSanctionsPressure;
}

export function SanctionsPressurePanel(props: SanctionsPressurePanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(SANCTIONS_PRESSURE_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadSanctionsPressure, [props.load]);
  const view = usePanelLoad<SanctionsPressureResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(SANCTIONS_PRESSURE_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={SANCTIONS_PRESSURE_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Sanctions pressure">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Sanctions pressure (24 h)</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<SanctionsPressureResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading sanctions…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Sanctions feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No additions.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} additions</div>
      <ul className="flex gap-2 flex-wrap" aria-label={`${r.byAuthority.length} authorities`}>
        {r.byAuthority.map(([authority, count]) => (
          <li
            key={authority}
            data-component="AuthorityChip"
            data-authority={authority}
            className="rounded border border-[var(--pellucid-border)] px-2 py-0.5 text-[10px] uppercase font-mono"
          >
            {authority}: <span data-field="count" className="text-[var(--pellucid-fg)]">{count}</span>
          </li>
        ))}
      </ul>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} sanction rows`}>
        {r.rows.map((row, i) => (
          <li
            key={`${row.authority}-${row.entity}-${i}`}
            data-component="SanctionRow"
            data-authority={row.authority}
            data-entity-type={row.entityType}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono uppercase text-[var(--pellucid-warn)]">{row.authority}</span>
            <span className="flex-1 px-2 truncate">{row.entity}</span>
            <span data-field="entity-type" className="font-mono text-[10px] text-[var(--pellucid-muted)]">{row.entityType}</span>
            <span data-field="jurisdiction" className="ml-2 font-mono uppercase">{row.jurisdiction}</span>
            <span data-field="listed-on" className="ml-2 text-[var(--pellucid-muted)]">{row.listedOn}</span>
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
  id: SANCTIONS_PRESSURE_PANEL_ID,
  title: "Sanctions pressure",
  blurb: "OFAC + EU + UK_HMT + UN recent additions.",
  component: SanctionsPressurePanel as React.ComponentType<unknown>,
  cacheKeys: ["sanctions:recent-additions:24h:v1"],
  minTier: 0,
  variants: "*",
});
