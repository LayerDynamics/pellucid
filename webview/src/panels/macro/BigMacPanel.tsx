import { useEffect, useMemo, useState, type ReactElement } from "react";

import { usePanelStore } from "../../state/usePanelStore";
import { loadBigMac, type BigMacResponse } from "../../data/loaders/macro/big-mac";
import { registerPanel } from "./index";
import { usePanelLoad } from "./usePanelLoad";
import { formatAssembledAtUtc, labelForCode } from "./labelForCode";

export const BIG_MAC_PANEL_ID = "macro/big-mac";
export const REQUIRED_TIER = 0;

export interface BigMacPanelProps {
  load?: typeof loadBigMac;
}

export function BigMacPanel(props: BigMacPanelProps): ReactElement {
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(BIG_MAC_PANEL_ID));
  const [iso, setIso] = useState<string>("");
  const loader = useMemo(() => props.load ?? loadBigMac, [props.load]);
  const view = usePanelLoad<BigMacResponse>({
    load: () => loader(iso ? { iso } : {}),
    requiredTier: REQUIRED_TIER,
    deps: [loader, iso],
  });
  useEffect(() => setLayout(BIG_MAC_PANEL_ID, { rowSpan: 2, colSpan: 3 }), [setLayout]);
  if (isHidden) return <></>;
  return (
    <section data-panel-id={BIG_MAC_PANEL_ID} className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3" aria-label="Big Mac index">
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Big Mac index</h3>
        <label className="flex items-center gap-1 text-[11px] uppercase font-mono text-[var(--pellucid-muted)]">
          <span>ISO</span>
          <input
            data-field="iso-input"
            type="text"
            value={iso}
            placeholder="any"
            aria-label="ISO filter"
            onChange={(e) => setIso(e.target.value)}
            className="rounded border border-[var(--pellucid-border)] bg-transparent px-2 py-0.5 text-[11px] uppercase focus:outline-none focus:border-[var(--pellucid-info)]"
          />
        </label>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ReturnType<typeof usePanelLoad<BigMacResponse>>): ReactElement {
  if (view.kind === "loading") return <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">Loading Big Mac…</div>;
  if (view.kind === "locked") return <div role="alert" data-state="locked" className="text-xs text-[var(--pellucid-warn)]">Locked — requires tier {view.minTier} or higher.</div>;
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") return <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]"><strong>Big Mac pipeline offline.</strong>{out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}</div>;
    return <div role="alert" data-state="error" data-error-code={out.code} className="text-xs text-[var(--pellucid-danger)]"><strong>{labelForCode(out.code)}</strong><br />{out.message}</div>;
  }
  const r = out.response;
  if (r.rows.length === 0) return <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">No countries match.</div>;
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <table className="w-full text-[11px]" data-component="BigMacTable" aria-label={`${r.rows.length} Big Mac rows`}>
        <thead className="text-[var(--pellucid-muted)]">
          <tr>
            <th className="text-left font-normal">ISO</th>
            <th className="text-left font-normal">Country</th>
            <th className="text-right font-normal">USD</th>
            <th className="text-right font-normal">Valuation</th>
          </tr>
        </thead>
        <tbody>
          {r.rows.map((row) => (
            <tr
              key={row.iso}
              data-component="BigMacRow"
              data-iso={row.iso}
              data-tone={row.valuationPct >= 0 ? "positive" : "negative"}
            >
              <td className="text-left font-mono">{row.iso}</td>
              <td className="text-left">{row.country}</td>
              <td className="text-right font-mono">${row.usdPrice.toFixed(2)}</td>
              <td
                data-field="valuation"
                className={`text-right font-mono ${row.valuationPct >= 0 ? "text-[var(--pellucid-success)]" : "text-[var(--pellucid-danger)]"}`}
              >
                {row.valuationPct >= 0 ? "+" : ""}{row.valuationPct.toFixed(1)}%
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        {r.rows.length} of {r.total} countries · {r.snapshotDate} · snapshot {formatAssembledAtUtc(r.assembledAtMs)}
        {r.stale ? <> · <span data-field="stale">cached</span></> : null}
      </footer>
    </div>
  );
}

registerPanel({
  id: BIG_MAC_PANEL_ID,
  title: "Big Mac index",
  blurb: "PPP-implied currency over/undervaluation.",
  component: BigMacPanel as React.ComponentType<unknown>,
  cacheKeys: ["economic:big-mac-index:v1"],
  minTier: 0,
  variants: "*",
});
