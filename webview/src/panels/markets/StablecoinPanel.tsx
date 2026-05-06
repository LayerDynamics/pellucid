import { useEffect, useState, type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadStablecoins,
  type StablecoinRow,
  type StablecoinsOutcome,
  type StablecoinsResponse,
} from "../../data/loaders/market/stablecoins";
import { registerPanel } from "./index";

export const STABLECOIN_PANEL_ID = "markets/stablecoins";
export const REQUIRED_TIER = 0;

export interface StablecoinPanelProps {
  load?: typeof loadStablecoins;
  bearerToken?: string;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: StablecoinsOutcome };

export function StablecoinPanel(props: StablecoinPanelProps): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(STABLECOIN_PANEL_ID));
  const [view, setView] = useState<ViewState>({ kind: "loading" });

  useEffect(() => {
    setLayout(STABLECOIN_PANEL_ID, { rowSpan: 2, colSpan: 2 });
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
    const loader = props.load ?? loadStablecoins;
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
      data-panel-id={STABLECOIN_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Stablecoin snapshot"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Stablecoins</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ViewState): ReactElement {
  if (view.kind === "loading") {
    return (
      <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">
        Loading stablecoins…
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
          <strong>Stablecoin pipeline offline.</strong>
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
  return <StablecoinBody response={out.response} />;
}

function StablecoinBody({
  response,
}: {
  response: StablecoinsResponse;
}): ReactElement {
  if (response.rows.length === 0) {
    return (
      <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">
        No stablecoin data.
      </div>
    );
  }
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <div
        data-component="StablecoinSummary"
        className="flex items-baseline justify-between text-[11px] text-[var(--pellucid-muted)]"
      >
        <span data-field="market-cap">
          Total cap: {formatBillions(response.totalMarketCapUsd)}
        </span>
        <span data-field="depeg-count">
          Depeg watch: {response.depegCount}
        </span>
      </div>
      <ul className="flex flex-col gap-1" aria-label={`${response.rows.length} stablecoins`}>
        {response.rows.map((r) => (
          <StablecoinRowView key={r.id} row={r} />
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

function StablecoinRowView({ row }: { row: StablecoinRow }): ReactElement {
  const tone = row.depegRisk ? "danger" : "neutral";
  return (
    <li
      data-component="StablecoinRow"
      data-symbol={row.symbol}
      data-tone={tone}
      className="flex items-baseline justify-between text-[11px]"
    >
      <span className="font-mono uppercase">{row.symbol}</span>
      <span className="font-mono text-[var(--pellucid-fg)]">
        ${row.usd.toFixed(4)}
      </span>
      <span
        data-field="peg-deviation"
        className={`font-mono ${row.depegRisk ? "text-[var(--pellucid-danger)]" : "text-[var(--pellucid-muted)]"}`}
      >
        {formatSignedPercent(row.pegDeviationPct)}
      </span>
      {row.depegRisk ? (
        <span data-field="depeg-badge" className="text-[10px] uppercase font-mono text-[var(--pellucid-danger)]">
          ⚠ depeg
        </span>
      ) : null}
    </li>
  );
}

export function formatBillions(usd: number): string {
  if (!Number.isFinite(usd) || usd <= 0) return "—";
  if (usd >= 1_000_000_000) return `$${(usd / 1_000_000_000).toFixed(2)}B`;
  if (usd >= 1_000_000) return `$${(usd / 1_000_000).toFixed(2)}M`;
  return `$${usd.toFixed(0)}`;
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
  id: STABLECOIN_PANEL_ID,
  title: "Stablecoins",
  blurb: "Stablecoin price + peg deviation watchlist.",
  component: StablecoinPanel as React.ComponentType<unknown>,
  cacheKeys: ["market:stablecoin-snapshot:v1"],
  minTier: 0,
  variants: "*",
});
