import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadInternetOutages, type OutagesResponse } from "../../data/loaders/infra/internet-outages";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const INTERNET_OUTAGES_PANEL_ID = "infra/internet-outages";
export const REQUIRED_TIER = 0;

export interface InternetOutagesPanelProps {
  load?: typeof loadInternetOutages;
}

export function InternetOutagesPanel(props: InternetOutagesPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(INTERNET_OUTAGES_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadInternetOutages, [props.load]);
  const view = usePanelLoad<OutagesResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(INTERNET_OUTAGES_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={INTERNET_OUTAGES_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Internet outages">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Internet outages</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<OutagesResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Outage feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No active outages.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} active</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} outages`}>
        {r.rows.map((row, i) => (
          <li
            key={`${row.provider}-${row.region}-${i}`}
            data-component="OutageRow"
            data-provider={row.provider}
            data-status={row.status}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono">{row.provider}</span>
            <span className="text-[var(--pellucid-muted)]">{row.region}</span>
            <span data-field="status" className="font-mono uppercase text-[var(--pellucid-warn)]">{row.status}</span>
            <span data-field="affected-as" className="ml-2 font-mono">{row.affectedAsCount} AS</span>
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
  id: INTERNET_OUTAGES_PANEL_ID,
  title: "Internet outages",
  blurb: "Provider/AS-level outage feed.",
  component: InternetOutagesPanel as React.ComponentType<unknown>,
  cacheKeys: ["infrastructure:grid-stress:current:v1"],
  minTier: 0,
  variants: "*",
});
