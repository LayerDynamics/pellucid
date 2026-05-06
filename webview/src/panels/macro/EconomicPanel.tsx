import { useMemo, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { useEffect } from "react";
import { loadEconomicSnapshot, type SnapshotResponse } from "../../data/loaders/macro/snapshot";
import { EconIndicatorTile, registerPanel } from "./index";
import { usePanelLoad } from "./usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "./labelForCode";

export const ECONOMIC_PANEL_ID = "macro/economic";
export const REQUIRED_TIER = 0;

export interface EconomicPanelProps {
  load?: typeof loadEconomicSnapshot;
}

export function EconomicPanel(props: EconomicPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(ECONOMIC_PANEL_ID));
  const loader = useMemo(() => props.load ?? loadEconomicSnapshot, [props.load]);
  const view = usePanelLoad<SnapshotResponse>({
    load: () => loader(),
    requiredTier: REQUIRED_TIER,
    deps: [loader],
  });
  useEffect(() => {
    setLayout(ECONOMIC_PANEL_ID, { rowSpan: 2, colSpan: 2 });
  }, [setLayout]);
  if (isHidden) return <></>;
  return (
    <section
      data-panel-id={ECONOMIC_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Economic snapshot"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Economic snapshot</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(
  view: ReturnType<typeof usePanelLoad<SnapshotResponse>>,
): ReactElement {
  if (view.kind === "loading") {
    return (
      <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">
        Loading indicators…
      </div>
    );
  }
  if (view.kind === "locked") {
    return (
      <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">
        Locked — requires tier {view.minTier} or higher.
      </div>
    );
  }
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") {
      return (
        <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]">
          <strong>Pipeline offline.</strong>
          {out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}
        </div>
      );
    }
    return (
      <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]">
        <strong>{labelForCode(out.code)}</strong>
        <br />
        {out.message}
      </div>
    );
  }
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <ul
        className="grid grid-cols-2 gap-2"
        aria-label={`${out.response.indicators.length} economic indicators`}
      >
        {out.response.indicators.map((ind) => (
          <li key={ind.code}>
            <EconIndicatorTile
              id={ind.code}
              code={ind.code}
              label={ind.code}
              value={ind.value.toFixed(2)}
              {...(ind.period ? { subline: ind.period } : {})}
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
  id: ECONOMIC_PANEL_ID,
  title: "Economic snapshot",
  blurb: "Latest FRED indicators (UNRATE, CPIAUCSL).",
  component: EconomicPanel as React.ComponentType<unknown>,
  cacheKeys: ["economic:fred-latest:UNRATE:v1", "economic:fred-latest:CPIAUCSL:v1"],
  minTier: 0,
  variants: "*",
});
