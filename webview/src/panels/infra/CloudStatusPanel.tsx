import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadCloudStatus, type CloudStatusResponse } from "../../data/loaders/infra/cloud-status";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const CLOUD_STATUS_PANEL_ID = "infra/cloud-status";
export const REQUIRED_TIER = 0;

export interface CloudStatusPanelProps {
  load?: typeof loadCloudStatus;
}

export function CloudStatusPanel(props: CloudStatusPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(CLOUD_STATUS_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadCloudStatus, [props.load]);
  const view = usePanelLoad<CloudStatusResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(CLOUD_STATUS_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={CLOUD_STATUS_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Cloud status">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Cloud provider status</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function statusClass(status: string): string {
  const sl = status.toLowerCase();
  if (["operational", "ok", "green", "available"].includes(sl)) return "text-[var(--pellucid-success)]";
  if (sl === "degraded") return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-danger)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<CloudStatusResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Cloud status feed offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No cloud data.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">
        Incidents: <span data-field="incident-count" className={`font-mono ${r.incidentCount === 0 ? "text-[var(--pellucid-success)]" : "text-[var(--pellucid-warn)]"}`}>{r.incidentCount}</span> / {r.total}
      </div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} cloud rows`}>
        {r.rows.map((row, i) => (
          <li
            key={`${row.provider}-${row.component}-${row.region}-${i}`}
            data-component="CloudStatusRow"
            data-provider={row.provider}
            data-status={row.status}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono uppercase">{row.provider}</span>
            <span className="flex-1 px-2 truncate">{row.component}</span>
            <span className="font-mono text-[var(--pellucid-muted)]">{row.region}</span>
            <span data-field="status" className={`ml-2 font-mono uppercase ${statusClass(row.status)}`}>{row.status}</span>
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
  id: CLOUD_STATUS_PANEL_ID,
  title: "Cloud provider status",
  blurb: "AWS / GCP / Azure component status snapshot.",
  component: CloudStatusPanel as React.ComponentType<unknown>,
  cacheKeys: ["technology:cloud-status:current:v1"],
  minTier: 0,
  variants: "*",
});
