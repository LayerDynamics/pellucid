import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadActiveCampaigns, type CampaignsResponse } from "../../data/loaders/infra/active-campaigns";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const ACTIVE_CAMPAIGNS_PANEL_ID = "infra/active-campaigns";
export const REQUIRED_TIER = 0;

export interface ActiveCampaignsPanelProps {
  load?: typeof loadActiveCampaigns;
}

export function ActiveCampaignsPanel(props: ActiveCampaignsPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(ACTIVE_CAMPAIGNS_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadActiveCampaigns, [props.load]);
  const view = usePanelLoad<CampaignsResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(ACTIVE_CAMPAIGNS_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={ACTIVE_CAMPAIGNS_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Active campaigns">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Active threat campaigns</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function severityClass(s: string): string {
  const sl = s.toLowerCase();
  if (sl === "critical" || sl === "extreme") return "text-[var(--pellucid-danger)]";
  if (sl === "high") return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<CampaignsResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Campaign feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No active campaigns.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">{r.total} active</div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} campaigns`}>
        {r.rows.map((row) => (
          <li
            key={row.id}
            data-component="CampaignRow"
            data-id={row.id}
            data-actor={row.actor}
            className="flex flex-col gap-0.5 text-[11px] border border-[var(--pellucid-border)] rounded p-1.5"
          >
            <div className="flex items-baseline justify-between">
              <span data-field="title" className="font-semibold">{row.title}</span>
              <span data-field="severity" className={`font-mono uppercase ${severityClass(row.severity)}`}>{row.severity}</span>
            </div>
            <div className="flex justify-between text-[10px] text-[var(--pellucid-muted)]">
              <span data-field="actor">Actor: <span className="font-mono">{row.actor}</span></span>
              <span data-field="sectors">{row.sectors.join(" · ")}</span>
            </div>
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
  id: ACTIVE_CAMPAIGNS_PANEL_ID,
  title: "Active threat campaigns",
  blurb: "Tracked APT/ransomware campaigns + impacted sectors.",
  component: ActiveCampaignsPanel as React.ComponentType<unknown>,
  cacheKeys: ["cyber:active-campaigns:v1"],
  minTier: 0,
  variants: "*",
});
