import { useEffect, useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadInfraSummary, type InfraSummaryResponse } from "../../data/loaders/infra/summary";
import { registerPanel } from "./index";
import { usePanelLoad } from "../macro/usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "../macro/labelForCode";

export const INFRA_PANEL_ID = "infra/summary";
export const REQUIRED_TIER = 0;

export interface InfraPanelProps {
  load?: typeof loadInfraSummary;
}

export function InfraPanel(props: InfraPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(INFRA_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadInfraSummary, [props.load]);
  const view = usePanelLoad<InfraSummaryResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => setLayout(INFRA_PANEL_ID, { rowSpan: 1, colSpan: 4 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={INFRA_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Infra summary">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Infra summary</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<InfraSummaryResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Infra pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <ul className="grid grid-cols-1 md:grid-cols-3 gap-2" aria-label={`${r.tiles.length} infra tiles`}>
        {r.tiles.map((t) => (
          <li
            key={t.code}
            data-component="InfraTile"
            data-code={t.code}
            data-tone={t.tone}
            className={`rounded border p-2 text-xs ${
              t.tone === "negative" ? "border-[var(--pellucid-danger)]" : t.tone === "positive" ? "border-[var(--pellucid-success)]" : "border-[var(--pellucid-border)]"
            }`}
          >
            <div className="flex items-baseline justify-between">
              <span className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">{t.code}</span>
              <span data-field="value" className="font-mono">{t.value}</span>
            </div>
            <div data-field="label">{t.label}</div>
          </li>
        ))}
      </ul>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        {r.availableTiles} signals · snapshot {formatAssembledAtUtc(r.assembledAtMs)}
        {r.stale ? <> · <span data-field="stale">cached</span></> : null}
      </footer>
    </div>
  );
}

registerPanel({
  id: INFRA_PANEL_ID,
  title: "Infra summary",
  blurb: "Cloud incidents + network outages + cyber severity.",
  component: InfraPanel as React.ComponentType<unknown>,
  cacheKeys: ["technology:cloud-status:current:v1", "infrastructure:grid-stress:current:v1", "cyber:incident-feed:24h:v1"],
  minTier: 0,
  variants: "*",
});
