import { useEffect, useState, type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadEarnings,
  type EarningsDayGroup,
  type EarningsEvent,
  type EarningsOutcome,
  type EarningsQuery,
  type EarningsResponse,
  type EarningsTiming,
} from "../../data/loaders/market/earnings";
import { registerPanel } from "./index";

export const EARNINGS_CALENDAR_PANEL_ID = "markets/earnings";
export const REQUIRED_TIER = 0;

export interface EarningsCalendarPanelProps {
  query?: EarningsQuery;
  load?: typeof loadEarnings;
  bearerToken?: string;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: EarningsOutcome };

export function EarningsCalendarPanel(
  props: EarningsCalendarPanelProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) =>
    s.isHidden(EARNINGS_CALENDAR_PANEL_ID),
  );

  const [view, setView] = useState<ViewState>({ kind: "loading" });

  useEffect(() => {
    setLayout(EARNINGS_CALENDAR_PANEL_ID, { rowSpan: 2, colSpan: 3 });
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
    const loader = props.load ?? loadEarnings;
    void loader(
      props.query ?? {},
      props.bearerToken ? { bearerToken: props.bearerToken } : {},
    ).then((outcome) => {
      if (cancelled) return;
      setView({ kind: "ready", outcome });
    });
    return () => {
      cancelled = true;
    };
  }, [hasTier, props.query, props.load, props.bearerToken]);

  if (isHidden) return <></>;

  return (
    <section
      data-panel-id={EARNINGS_CALENDAR_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Earnings calendar"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Earnings calendar</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ViewState): ReactElement {
  if (view.kind === "loading") {
    return (
      <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">
        Loading earnings…
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
          <strong>Earnings pipeline offline.</strong>
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
  return <CalendarBody response={out.response} />;
}

function CalendarBody({ response }: { response: EarningsResponse }): ReactElement {
  if (response.days.length === 0) {
    return (
      <div data-state="empty" role="status" className="text-xs text-[var(--pellucid-muted)]">
        No upcoming earnings in the next {response.lookaheadDays} days.
      </div>
    );
  }
  return (
    <div data-state="ready" className="flex flex-col gap-3">
      {response.days.map((d) => (
        <DayGroup key={d.date} group={d} />
      ))}
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        {response.total} events · {response.lookaheadDays}d lookahead · snapshot{" "}
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

function DayGroup({ group }: { group: EarningsDayGroup }): ReactElement {
  return (
    <article
      data-component="EarningsDayGroup"
      data-date={group.date}
      className="rounded border border-[var(--pellucid-border)] p-2"
    >
      <header className="mb-1 text-[11px] font-semibold uppercase tracking-wide">
        {group.date}
      </header>
      <ul className="flex flex-col gap-1">
        {group.events.map((e) => (
          <li
            key={`${e.symbol}-${e.date}`}
            data-component="EarningsRow"
            data-symbol={e.symbol}
            className="flex items-baseline justify-between text-[11px]"
          >
            <span className="font-mono uppercase">{e.symbol}</span>
            <span className="flex-1 px-2 text-[var(--pellucid-muted)] truncate">
              {e.company}
            </span>
            <span data-field="timing" className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">
              {timingLabel(e.timing)}
            </span>
            {typeof e.epsEstimate === "number" ? (
              <span data-field="eps-estimate" className="ml-2 font-mono text-[var(--pellucid-fg)]">
                est ${e.epsEstimate.toFixed(2)}
              </span>
            ) : null}
          </li>
        ))}
      </ul>
    </article>
  );
}

export function timingLabel(t: EarningsTiming): string {
  switch (t) {
    case "before-open":
      return "BMO";
    case "after-close":
      return "AMC";
    case "during-hours":
      return "INT";
    default:
      return "—";
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
  id: EARNINGS_CALENDAR_PANEL_ID,
  title: "Earnings calendar",
  blurb: "Upcoming earnings releases across the basket.",
  component: EarningsCalendarPanel as React.ComponentType<unknown>,
  cacheKeys: ["market:earnings-calendar:7d:v1"],
  minTier: 0,
  variants: "*",
});

// Re-export types so panel + test sit at the same call surface.
export type { EarningsEvent, EarningsResponse };
