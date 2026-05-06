import { useEffect, useState, type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadDailyBrief,
  type BriefLine,
  type DailyBriefOutcome,
  type DailyBriefResponse,
  type DayTone,
} from "../../data/loaders/market/daily-brief";
import { registerPanel } from "./index";

export const DAILY_MARKET_BRIEF_PANEL_ID = "markets/daily-brief";
export const REQUIRED_TIER = 0;

export interface DailyMarketBriefPanelProps {
  load?: typeof loadDailyBrief;
  bearerToken?: string;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: DailyBriefOutcome };

export function DailyMarketBriefPanel(
  props: DailyMarketBriefPanelProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) =>
    s.isHidden(DAILY_MARKET_BRIEF_PANEL_ID),
  );
  const [view, setView] = useState<ViewState>({ kind: "loading" });

  useEffect(() => {
    setLayout(DAILY_MARKET_BRIEF_PANEL_ID, { rowSpan: 2, colSpan: 2 });
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
    const loader = props.load ?? loadDailyBrief;
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
      data-panel-id={DAILY_MARKET_BRIEF_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Daily market brief"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Daily brief</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ViewState): ReactElement {
  if (view.kind === "loading") {
    return (
      <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">
        Composing brief…
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
          <strong>Brief pipeline offline.</strong>
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
  return <BriefBody response={out.response} />;
}

function BriefBody({ response }: { response: DailyBriefResponse }): ReactElement {
  if (response.lines.length === 0) {
    return (
      <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">
        No brief data available.
      </div>
    );
  }
  return (
    <div data-state="ready" className="flex flex-col gap-2">
      <ul className="flex flex-col gap-2" aria-label={`${response.lines.length} brief lines`}>
        {response.lines.map((l) => (
          <LineRow key={l.section} line={l} />
        ))}
      </ul>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        Composed: {formatAssembledAtUtc(response.assembledAtMs)}
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

function LineRow({ line }: { line: BriefLine }): ReactElement {
  return (
    <li
      data-component="BriefLine"
      data-section={line.section}
      data-tone={line.tone}
      className="flex flex-col gap-0.5 rounded border border-[var(--pellucid-border)] p-2"
    >
      <div className="flex items-baseline justify-between gap-2">
        <span data-field="headline" className={`text-xs font-semibold ${toneClass(line.tone)}`}>
          {line.headline}
        </span>
        <span className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">
          {line.section}
        </span>
      </div>
      <span data-field="rationale" className="text-[11px] text-[var(--pellucid-muted)]">
        {line.rationale}
      </span>
    </li>
  );
}

export function toneClass(tone: DayTone): string {
  switch (tone) {
    case "strong-up":
    case "up":
      return "text-[var(--pellucid-success)]";
    case "strong-down":
    case "down":
      return "text-[var(--pellucid-danger)]";
    default:
      return "text-[var(--pellucid-fg)]";
  }
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
  id: DAILY_MARKET_BRIEF_PANEL_ID,
  title: "Daily brief",
  blurb: "Composed daily market summary across stocks/crypto/commodities/VIX.",
  component: DailyMarketBriefPanel as React.ComponentType<unknown>,
  cacheKeys: [
    "market:stocks-bootstrap:v1",
    "market:crypto-snapshot:v1",
    "market:commodities-snapshot:v1",
  ],
  minTier: 0,
  variants: "*",
});
