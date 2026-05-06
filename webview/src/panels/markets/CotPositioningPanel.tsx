import { useEffect, useState, type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadCot,
  type CotOutcome,
  type CotResponse,
  type CotRow,
  type SortMode,
} from "../../data/loaders/market/cot";
import { registerPanel } from "./index";

/** Stable id for `usePanelStore` registration. */
export const COT_POSITIONING_PANEL_ID = "markets/cot";

/** Anonymous-tier handler. */
export const REQUIRED_TIER = 0;

/** Default sort the panel opens with. */
export const DEFAULT_SORT: SortMode = "managed-money-net-desc";

const SORT_LABELS: Record<SortMode, string> = {
  "managed-money-net-desc": "Managed-money net (most-bullish)",
  "managed-money-net-asc": "Managed-money net (most-bearish)",
  "open-interest-desc": "Open interest (largest)",
  "name-asc": "Contract name (A → Z)",
};

const SORT_OPTIONS: SortMode[] = [
  "managed-money-net-desc",
  "managed-money-net-asc",
  "open-interest-desc",
  "name-asc",
];

export interface CotPositioningPanelProps {
  /** Initial sort. */
  sort?: SortMode;
  /** Override the loader (testing). */
  load?: typeof loadCot;
  /** Optional bearer token. */
  bearerToken?: string;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: CotOutcome };

/**
 * COT positioning panel — M3 family 4.2, T4.2.7.
 *
 * Renders the SLOW-tier `market:cot-report:weekly:v1` snapshot
 * the `seed_cot` seeder publishes from CFTC's Disaggregated COT
 * report. Each row shows the report week, contract name, open
 * interest, and the four standout positioning numbers
 * (managed-money long / short / net / net-as-%-of-OI). The
 * managed-money-net column is tone-coded so a user can scan
 * for bullish vs bearish positioning at a glance.
 */
export function CotPositioningPanel(
  props: CotPositioningPanelProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(COT_POSITIONING_PANEL_ID));

  const [sort, setSort] = useState<SortMode>(props.sort ?? DEFAULT_SORT);
  const [view, setView] = useState<ViewState>({ kind: "loading" });

  useEffect(() => {
    setLayout(COT_POSITIONING_PANEL_ID, { rowSpan: 2, colSpan: 3 });
  }, [setLayout]);

  useEffect(() => {
    let cancelled = false;
    if (!hasTier(REQUIRED_TIER)) {
      setView({ kind: "locked", minTier: REQUIRED_TIER });
      return () => {
        cancelled = true;
      };
    }
    setView({ kind: "loading" });
    const loader = props.load ?? loadCot;
    void loader(
      { sort },
      props.bearerToken ? { bearerToken: props.bearerToken } : {},
    ).then((outcome) => {
      if (cancelled) return;
      setView({ kind: "ready", outcome });
    });
    return () => {
      cancelled = true;
    };
  }, [hasTier, sort, props.load, props.bearerToken]);

  if (isHidden) return <></>;

  return (
    <section
      data-panel-id={COT_POSITIONING_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="COT positioning"
    >
      <header className="flex items-baseline justify-between gap-3">
        <h3 className="text-sm font-semibold">COT positioning</h3>
        <SortControl current={sort} onChange={setSort} />
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ViewState): ReactElement {
  if (view.kind === "loading") {
    return (
      <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">
        Loading positioning…
      </div>
    );
  }
  if (view.kind === "locked") {
    return (
      <div
        role="alert"
        data-state="locked"
        className="text-xs text-[var(--pellucid-warn)]"
      >
        Locked — requires tier {view.minTier} or higher.
      </div>
    );
  }
  const out = view.outcome;
  if (out.kind === "error") {
    if (out.code === "bootstrap_upstream_empty") {
      return (
        <div role="alert" data-state="outage" className="text-xs text-[var(--pellucid-warn)]">
          <strong>COT pipeline offline.</strong>
          {out.retryAfterSecs ? <> Retry in {out.retryAfterSecs}s.</> : null}
        </div>
      );
    }
    return (
      <div
        role="alert"
        data-state="error"
        data-error-code={out.code}
        className="text-xs text-[var(--pellucid-danger)]"
      >
        <strong>{labelForCode(out.code)}</strong>
        <br />
        {out.message}
      </div>
    );
  }
  return <CotTable response={out.response} />;
}

function CotTable({ response }: { response: CotResponse }): ReactElement {
  if (response.rows.length === 0) {
    return (
      <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">
        No positioning rows match the current filter.
      </div>
    );
  }
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <table
        className="w-full text-[11px]"
        data-component="CotTable"
        aria-label={`${response.rows.length} COT positioning rows`}
      >
        <thead className="text-[var(--pellucid-muted)]">
          <tr>
            <th className="text-left font-normal">Contract</th>
            <th className="text-right font-normal">Open interest</th>
            <th className="text-right font-normal">MM long</th>
            <th className="text-right font-normal">MM short</th>
            <th className="text-right font-normal">MM net</th>
            <th className="text-right font-normal">% OI</th>
          </tr>
        </thead>
        <tbody>
          {response.rows.map((r) => (
            <CotTableRow key={r.contractCode} row={r} />
          ))}
        </tbody>
      </table>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        Showing {response.rows.length} of {response.total} contracts · report{" "}
        {response.rows[0]?.reportDate ?? "—"} · snapshot{" "}
        {formatAssembledAtUtc(response.assembledAtMs)}
        {response.stale ? (
          <>
            {" "}
            · <span data-field="stale">cached snapshot</span>
          </>
        ) : null}
      </footer>
    </div>
  );
}

function CotTableRow({ row }: { row: CotRow }): ReactElement {
  return (
    <tr
      data-component="CotTableRow"
      data-contract-code={row.contractCode}
      data-tone={netTone(row.managedMoneyNet)}
    >
      <td className="text-left font-mono uppercase">{row.contractName}</td>
      <td className="text-right font-mono">
        {formatThousands(row.openInterestAll)}
      </td>
      <td className="text-right font-mono">
        {formatThousands(row.managedMoneyLong)}
      </td>
      <td className="text-right font-mono">
        {formatThousands(row.managedMoneyShort)}
      </td>
      <td
        data-field="managed-money-net"
        className={`text-right font-mono ${netClass(row.managedMoneyNet)}`}
      >
        {formatSignedThousands(row.managedMoneyNet)}
      </td>
      <td
        data-field="managed-money-pct-oi"
        className={`text-right font-mono ${netClass(row.managedMoneyNet)}`}
      >
        {formatSignedPercent(row.managedMoneyNetPctOi)}
      </td>
    </tr>
  );
}

function SortControl({
  current,
  onChange,
}: {
  current: SortMode;
  onChange: (next: SortMode) => void;
}): ReactElement {
  return (
    <label className="flex items-center gap-1 text-[11px]">
      <span className="text-[var(--pellucid-muted)]">Sort</span>
      <select
        value={current}
        aria-label="Sort mode"
        data-field="sort-select"
        className="rounded border border-[var(--pellucid-border)] bg-transparent px-1 py-0.5 text-[var(--pellucid-fg)]"
        onChange={(e) => onChange(e.target.value as SortMode)}
      >
        {SORT_OPTIONS.map((mode) => (
          <option key={mode} value={mode}>
            {SORT_LABELS[mode]}
          </option>
        ))}
      </select>
    </label>
  );
}

/** Tone for a managed-money-net value. Pure, exported for tests. */
export function netTone(value: number): "positive" | "negative" | "neutral" {
  if (!Number.isFinite(value) || value === 0) return "neutral";
  return value > 0 ? "positive" : "negative";
}

function netClass(value: number): string {
  switch (netTone(value)) {
    case "positive":
      return "text-[var(--pellucid-success)]";
    case "negative":
      return "text-[var(--pellucid-danger)]";
    default:
      return "text-[var(--pellucid-fg)]";
  }
}

/** Format an integer with thousands grouping. Pure, exported. */
export function formatThousands(value: number): string {
  if (!Number.isFinite(value)) return "—";
  return value.toLocaleString("en-US");
}

/** Format a signed integer with thousands grouping + leading sign.
 *  Pure, exported. */
export function formatSignedThousands(value: number): string {
  if (!Number.isFinite(value)) return "—";
  if (value === 0) return "0";
  const sign = value < 0 ? "-" : "+";
  return `${sign}${Math.abs(value).toLocaleString("en-US")}`;
}

/** Format a signed % to 1 decimal. Pure, exported. */
export function formatSignedPercent(value: number): string {
  if (!Number.isFinite(value)) return "—";
  if (value === 0) return "0.0%";
  const sign = value < 0 ? "" : "+";
  return `${sign}${value.toFixed(1)}%`;
}

function labelForCode(code: string): string {
  switch (code) {
    case "invalid_request":
      return "Invalid request";
    case "cache_failure":
      return "Cache failure";
    case "cache_shape":
      return "Wire-shape mismatch";
    case "network":
      return "Network error";
    default:
      return "Error";
  }
}

/** Format wall-clock ms as `HH:MM UTC`. Pure, exported. */
export function formatAssembledAtUtc(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const d = new Date(ms);
  const hh = String(d.getUTCHours()).padStart(2, "0");
  const mm = String(d.getUTCMinutes()).padStart(2, "0");
  return `${hh}:${mm} UTC`;
}

registerPanel({
  id: COT_POSITIONING_PANEL_ID,
  title: "COT positioning",
  blurb: "Weekly CFTC commitments-of-traders managed-money positioning.",
  component: CotPositioningPanel as React.ComponentType<unknown>,
  cacheKeys: ["market:cot-report:weekly:v1"],
  minTier: 0,
  variants: "*",
});
