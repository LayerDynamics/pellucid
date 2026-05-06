import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadCveTrending, type CveTrendingResponse } from "../../data/loaders/infra/cve-trending";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const CVE_TRENDING_PANEL_ID = "infra/cve-trending";
export const REQUIRED_TIER = 0;

export interface CveTrendingPanelProps {
  load?: typeof loadCveTrending;
}

export function CveTrendingPanel(props: CveTrendingPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(CVE_TRENDING_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadCveTrending, [props.load]);
  const view = usePanelLoad<CveTrendingResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(CVE_TRENDING_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={CVE_TRENDING_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Trending CVEs">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Trending CVEs</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function cvssClass(score: number): string {
  if (score >= 9) return "text-[var(--pellucid-danger)]";
  if (score >= 7) return "text-[var(--pellucid-warn)]";
  return "text-[var(--pellucid-muted)]";
}

function renderBody(view: ReturnType<typeof usePanelLoad<CveTrendingResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading CVEs…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>NVD pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No trending CVEs.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div className="text-xs">
        Max CVSS: <span data-field="max-cvss" className={`font-mono ${cvssClass(r.maxCvss)}`}>{r.maxCvss.toFixed(1)}</span>
      </div>
      <ul className="flex flex-col gap-1" aria-label={`${r.rows.length} CVEs`}>
        {r.rows.map((row) => (
          <li
            key={row.cveId}
            data-component="CveRow"
            data-cve={row.cveId}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span data-field="cve-id" className="font-mono uppercase">{row.cveId}</span>
            <span className="flex-1 px-2 truncate">{row.summary}</span>
            <span data-field="cvss" className={`ml-2 font-mono ${cvssClass(row.cvssScore)}`}>{row.cvssScore.toFixed(1)}</span>
            <span data-field="severity" className="ml-2 font-mono text-[var(--pellucid-muted)]">{row.severity}</span>
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
  id: CVE_TRENDING_PANEL_ID,
  title: "Trending CVEs",
  blurb: "Highest-CVSS CVEs from the trending feed.",
  component: CveTrendingPanel as React.ComponentType<unknown>,
  cacheKeys: ["cyber:cve-trending:v1"],
  minTier: 0,
  variants: "*",
});
