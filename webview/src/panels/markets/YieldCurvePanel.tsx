import { useEffect, useState, type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadYieldCurve,
  type YieldCurveOutcome,
  type YieldCurveResponse,
  type YieldPoint,
} from "../../data/loaders/market/yield-curve";
import { registerPanel } from "./index";

export const YIELD_CURVE_PANEL_ID = "markets/yield-curve";
export const REQUIRED_TIER = 0;

export interface YieldCurvePanelProps {
  load?: typeof loadYieldCurve;
  bearerToken?: string;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: YieldCurveOutcome };

export function YieldCurvePanel(props: YieldCurvePanelProps): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(YIELD_CURVE_PANEL_ID));
  const [view, setView] = useState<ViewState>({ kind: "loading" });

  useEffect(() => {
    setLayout(YIELD_CURVE_PANEL_ID, { rowSpan: 2, colSpan: 3 });
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
    const loader = props.load ?? loadYieldCurve;
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
      data-panel-id={YIELD_CURVE_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Treasury yield curve"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Yield curve</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ViewState): ReactElement {
  if (view.kind === "loading") {
    return (
      <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">
        Loading yield curve…
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
          <strong>Yield curve pipeline offline.</strong>
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
  return <CurveBody response={out.response} />;
}

function CurveBody({ response }: { response: YieldCurveResponse }): ReactElement {
  return (
    <div data-state="ready" className="flex flex-col gap-3">
      <CurveSparkline points={response.points} />
      <SpreadStrip response={response} />
      <PointsTable points={response.points} />
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        {response.points.length} points · snapshot{" "}
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

const SVG_W = 320;
const SVG_H = 64;

function CurveSparkline({ points }: { points: YieldPoint[] }): ReactElement {
  if (points.length < 2) {
    return (
      <div data-component="YieldSparkline" data-state="too-few-points" className="text-[11px] text-[var(--pellucid-muted)]">
        Not enough points to chart.
      </div>
    );
  }
  const sorted = [...points].sort((a, b) => a.maturityMonths - b.maturityMonths);
  const minMonths = sorted[0]!.maturityMonths;
  const maxMonths = sorted[sorted.length - 1]!.maturityMonths;
  const yields = sorted.map((p) => p.yieldPct);
  const minY = Math.min(...yields);
  const maxY = Math.max(...yields);
  const xRange = Math.max(1, maxMonths - minMonths);
  const yRange = Math.max(0.001, maxY - minY);
  const path = sorted
    .map((p, i) => {
      const x = ((p.maturityMonths - minMonths) / xRange) * SVG_W;
      const y = SVG_H - ((p.yieldPct - minY) / yRange) * SVG_H;
      return `${i === 0 ? "M" : "L"} ${x.toFixed(1)} ${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg
      data-component="YieldSparkline"
      width={SVG_W}
      height={SVG_H}
      viewBox={`0 0 ${SVG_W} ${SVG_H}`}
      role="img"
      aria-label="Yield curve sparkline"
    >
      <path
        data-field="curve-path"
        d={path}
        fill="none"
        stroke="currentColor"
        strokeWidth={1.5}
      />
    </svg>
  );
}

function SpreadStrip({ response }: { response: YieldCurveResponse }): ReactElement {
  const items: Array<{ id: string; label: string; value: number | undefined }> = [
    { id: "ten-minus-two", label: "10y-2y", value: response.spreads.tenMinusTwo },
    {
      id: "ten-minus-three-month",
      label: "10y-3m",
      value: response.spreads.tenMinusThreeMonth,
    },
    {
      id: "thirty-minus-five",
      label: "30y-5y",
      value: response.spreads.thirtyMinusFive,
    },
  ];
  return (
    <div
      data-component="SpreadStrip"
      data-inverted={response.inverted ? "true" : "false"}
      className="flex flex-wrap items-center gap-3 text-[11px] font-mono"
    >
      {items.map((it) => (
        <span
          key={it.id}
          data-spread={it.id}
          className={`${spreadClass(it.value)} font-mono`}
        >
          {it.label}: {formatSpread(it.value)}
        </span>
      ))}
      {response.inverted ? (
        <span data-field="inverted-badge" className="text-[var(--pellucid-danger)]">
          ⚠ inverted
        </span>
      ) : null}
    </div>
  );
}

function PointsTable({ points }: { points: YieldPoint[] }): ReactElement {
  return (
    <table
      data-component="YieldPointsTable"
      className="w-full text-[11px]"
      aria-label={`${points.length} yield curve points`}
    >
      <thead className="text-[var(--pellucid-muted)]">
        <tr>
          <th className="text-left font-normal">Maturity</th>
          <th className="text-right font-normal">Yield</th>
        </tr>
      </thead>
      <tbody>
        {points.map((p) => (
          <tr
            key={p.seriesCode}
            data-component="YieldPointRow"
            data-series-code={p.seriesCode}
          >
            <td className="text-left font-mono">{p.maturityLabel}</td>
            <td className="text-right font-mono">{p.yieldPct.toFixed(2)}%</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function formatSpread(v: number | undefined): string {
  if (typeof v !== "number" || !Number.isFinite(v)) return "—";
  const sign = v < 0 ? "" : "+";
  return `${sign}${v.toFixed(2)}%`;
}

function spreadClass(v: number | undefined): string {
  if (typeof v !== "number" || !Number.isFinite(v)) return "text-[var(--pellucid-muted)]";
  return v < 0 ? "text-[var(--pellucid-danger)]" : "text-[var(--pellucid-fg)]";
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
  id: YIELD_CURVE_PANEL_ID,
  title: "Yield curve",
  blurb: "U.S. Treasury constant-maturity yield curve + inversion spreads.",
  component: YieldCurvePanel as React.ComponentType<unknown>,
  cacheKeys: ["market:yield-curve:treasury:v1"],
  minTier: 0,
  variants: "*",
});
