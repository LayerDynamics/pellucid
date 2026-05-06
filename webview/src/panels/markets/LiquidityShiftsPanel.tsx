import { useEffect, useState, type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadLiquidityShifts,
  type LiquiditySeries,
  type LiquidityShiftsOutcome,
  type LiquidityShiftsResponse,
} from "../../data/loaders/market/liquidity-shifts";
import { registerPanel } from "./index";

export const LIQUIDITY_SHIFTS_PANEL_ID = "markets/liquidity-shifts";
export const REQUIRED_TIER = 0;

export interface LiquidityShiftsPanelProps {
  load?: typeof loadLiquidityShifts;
  bearerToken?: string;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: LiquidityShiftsOutcome };

export function LiquidityShiftsPanel(
  props: LiquidityShiftsPanelProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) =>
    s.isHidden(LIQUIDITY_SHIFTS_PANEL_ID),
  );
  const [view, setView] = useState<ViewState>({ kind: "loading" });

  useEffect(() => {
    setLayout(LIQUIDITY_SHIFTS_PANEL_ID, { rowSpan: 2, colSpan: 2 });
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
    const loader = props.load ?? loadLiquidityShifts;
    void loader(
      props.bearerToken ? { bearerToken: props.bearerToken } : {},
    ).then((outcome) => {
      if (cancelled) return;
      setView({ kind: "ready", outcome });
    });
    return () => {
      cancelled = true;
    };
  }, [hasTier, props.load, props.bearerToken]);

  if (isHidden) return <></>;

  return (
    <section
      data-panel-id={LIQUIDITY_SHIFTS_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Liquidity shifts"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Liquidity shifts</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ViewState): ReactElement {
  if (view.kind === "loading") {
    return (
      <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">
        Loading liquidity…
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
          <strong>Liquidity pipeline offline.</strong>
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
  return <LiquidityBody response={out.response} />;
}

function LiquidityBody({
  response,
}: {
  response: LiquidityShiftsResponse;
}): ReactElement {
  return (
    <div data-state="ready" className="flex flex-col gap-3">
      <div
        data-component="NetLiquidity"
        className="flex flex-col"
      >
        <span className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">
          Net liquidity (WALCL − RRP)
        </span>
        <span data-field="net-liquidity" className="text-2xl font-mono font-semibold">
          {formatBillions(response.netLiquidityBillionUsd)}
        </span>
      </div>
      <ul className="flex flex-col gap-1" aria-label={`${response.series.length} liquidity series`}>
        {response.series.map((s) => (
          <LiquidityRow key={s.seriesCode} series={s} />
        ))}
      </ul>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        Snapshot: {formatAssembledAtUtc(response.assembledAtMs)}
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

function LiquidityRow({ series }: { series: LiquiditySeries }): ReactElement {
  const tone = deltaTone(series.periodDelta);
  return (
    <li
      data-component="LiquidityRow"
      data-series-code={series.seriesCode}
      data-tone={tone}
      className="flex items-baseline justify-between text-[11px]"
    >
      <span className="font-mono">{series.seriesCode}</span>
      <span data-field="latest-value" className="font-mono text-[var(--pellucid-fg)]">
        {formatBillions(series.latestValue)}
      </span>
      <span
        data-field="period-delta"
        className={`font-mono ${
          tone === "positive"
            ? "text-[var(--pellucid-success)]"
            : tone === "negative"
              ? "text-[var(--pellucid-danger)]"
              : "text-[var(--pellucid-muted)]"
        }`}
      >
        {formatSignedDelta(series.periodDelta)}
        {typeof series.periodDeltaPct === "number" &&
        Number.isFinite(series.periodDeltaPct) ? (
          <> ({formatSignedPercent(series.periodDeltaPct)})</>
        ) : null}
      </span>
    </li>
  );
}

export function deltaTone(
  v: number | undefined,
): "positive" | "negative" | "neutral" {
  if (typeof v !== "number" || !Number.isFinite(v) || v === 0) return "neutral";
  return v > 0 ? "positive" : "negative";
}

export function formatBillions(usd: number): string {
  if (!Number.isFinite(usd)) return "—";
  if (usd === 0) return "$0B";
  const sign = usd < 0 ? "-" : "";
  const abs = Math.abs(usd);
  // Values are already in $B (FRED publishes in $B for these series).
  if (abs >= 1_000) return `${sign}$${(abs / 1_000).toFixed(2)}T`;
  return `${sign}$${abs.toFixed(0)}B`;
}

export function formatSignedDelta(v: number | undefined): string {
  if (typeof v !== "number" || !Number.isFinite(v)) return "—";
  if (v === 0) return "$0B";
  const sign = v < 0 ? "-" : "+";
  const abs = Math.abs(v);
  if (abs >= 1_000) return `${sign}$${(abs / 1_000).toFixed(2)}T`;
  return `${sign}$${abs.toFixed(0)}B`;
}

export function formatSignedPercent(v: number): string {
  if (!Number.isFinite(v)) return "—";
  if (v === 0) return "0.00%";
  const sign = v < 0 ? "" : "+";
  return `${sign}${v.toFixed(2)}%`;
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

export function formatAssembledAtUtc(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const d = new Date(ms);
  const hh = String(d.getUTCHours()).padStart(2, "0");
  const mm = String(d.getUTCMinutes()).padStart(2, "0");
  return `${hh}:${mm} UTC`;
}

registerPanel({
  id: LIQUIDITY_SHIFTS_PANEL_ID,
  title: "Liquidity shifts",
  blurb: "Fed balance sheet, M2, RRP — net USD liquidity proxy.",
  component: LiquidityShiftsPanel as React.ComponentType<unknown>,
  cacheKeys: ["market:liquidity-shifts:v1"],
  minTier: 0,
  variants: "*",
});
