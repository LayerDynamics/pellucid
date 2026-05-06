import { useEffect, useState, type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadFearGreed,
  type Component,
  type FearGreedOutcome,
  type FearGreedResponse,
  type SentimentLabel,
} from "../../data/loaders/market/fear-greed";
import { registerPanel } from "./index";

/** Stable id for `usePanelStore` registration. */
export const FEAR_GREED_PANEL_ID = "markets/fear-greed";

/** Anonymous-tier handler. */
export const REQUIRED_TIER = 0;

export interface FearGreedPanelProps {
  /** Override the loader (testing). */
  load?: typeof loadFearGreed;
  /** Optional bearer token. */
  bearerToken?: string;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: FearGreedOutcome };

/**
 * Fear & greed panel — M3 family 4.2, T4.2.6.
 *
 * Renders the composite 0–100 fear/greed score the handler
 * derives from the existing FAST-tier market cache slots. The
 * dial is a horizontal gradient bar (red → amber → green) with
 * a marker arrow at `score`; per-component chips break the
 * composite into volatility / momentum / strength / volume so a
 * user can see WHY the dial sits where it does.
 */
export function FearGreedPanel(props: FearGreedPanelProps): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(FEAR_GREED_PANEL_ID));

  const [view, setView] = useState<ViewState>({ kind: "loading" });

  useEffect(() => {
    setLayout(FEAR_GREED_PANEL_ID, { rowSpan: 2, colSpan: 2 });
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
    const loader = props.load ?? loadFearGreed;
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
      data-panel-id={FEAR_GREED_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Fear & greed"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Fear &amp; greed</h3>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ViewState): ReactElement {
  if (view.kind === "loading") {
    return (
      <div
        role="status"
        aria-live="polite"
        className="text-xs text-[var(--pellucid-muted)]"
      >
        Loading sentiment…
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
        <div
          role="alert"
          data-state="outage"
          className="text-xs text-[var(--pellucid-warn)]"
        >
          <strong>Sentiment pipeline offline.</strong>
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
  return <FearGreedReady response={out.response} />;
}

function FearGreedReady({
  response,
}: {
  response: FearGreedResponse;
}): ReactElement {
  return (
    <div data-state="ready" className="flex flex-col gap-3">
      <div
        data-component="FearGreedDial"
        data-score={response.score}
        data-label={response.label}
        className="flex flex-col gap-1"
      >
        <div className="flex items-baseline justify-between">
          <span
            data-field="composite-score"
            className="text-2xl font-semibold font-mono"
          >
            {response.score}
          </span>
          <span
            data-field="composite-label"
            className={`text-[11px] uppercase font-mono tracking-wide ${labelClass(
              response.label,
            )}`}
          >
            {labelText(response.label)}
          </span>
        </div>
        <DialBar score={response.score} />
      </div>
      <ul
        data-component="FearGreedComponents"
        aria-label={`${response.components.length} sentiment components`}
        className="flex flex-col gap-1"
      >
        {response.components.map((c) => (
          <ComponentRow key={c.name} component={c} />
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

function DialBar({ score }: { score: number }): ReactElement {
  const clamped = Math.max(0, Math.min(100, score));
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={clamped}
      data-component="FearGreedDialBar"
      className="relative h-2 w-full overflow-hidden rounded-full"
      style={{
        background:
          "linear-gradient(to right, var(--pellucid-danger), var(--pellucid-warn), var(--pellucid-success))",
      }}
    >
      <div
        data-field="dial-marker"
        className="absolute top-1/2 h-3 w-0.5 -translate-x-1/2 -translate-y-1/2 bg-[var(--pellucid-fg)]"
        style={{ left: `${clamped}%` }}
      />
    </div>
  );
}

function ComponentRow({
  component,
}: {
  component: Component;
}): ReactElement {
  return (
    <li
      data-component="FearGreedComponentRow"
      data-component-name={component.name}
      data-component-label={component.label}
      className="flex items-center justify-between text-[11px]"
    >
      <span data-field="component-name" className="font-mono uppercase">
        {component.name}
      </span>
      <span
        data-field="component-rationale"
        className="text-[var(--pellucid-muted)]"
      >
        {component.rationale}
      </span>
      <span
        data-field="component-score"
        className={`font-mono ${labelClass(component.label)}`}
      >
        {component.score}
      </span>
    </li>
  );
}

/** Map a label to a tone class. Pure, exported for tests. */
export function labelClass(label: SentimentLabel): string {
  switch (label) {
    case "extreme-fear":
    case "fear":
      return "text-[var(--pellucid-danger)]";
    case "extreme-greed":
    case "greed":
      return "text-[var(--pellucid-success)]";
    default:
      return "text-[var(--pellucid-muted)]";
  }
}

/** Display string for a label. Pure, exported for tests. */
export function labelText(label: SentimentLabel): string {
  switch (label) {
    case "extreme-fear":
      return "Extreme fear";
    case "fear":
      return "Fear";
    case "neutral":
      return "Neutral";
    case "greed":
      return "Greed";
    case "extreme-greed":
      return "Extreme greed";
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

/** Format wall-clock ms as `HH:MM UTC`. Pure, exported for tests. */
export function formatAssembledAtUtc(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const d = new Date(ms);
  const hh = String(d.getUTCHours()).padStart(2, "0");
  const mm = String(d.getUTCMinutes()).padStart(2, "0");
  return `${hh}:${mm} UTC`;
}

registerPanel({
  id: FEAR_GREED_PANEL_ID,
  title: "Fear & greed",
  blurb: "Composite sentiment dial built from market cache slots.",
  component: FearGreedPanel as React.ComponentType<unknown>,
  cacheKeys: ["market:stocks-bootstrap:v1", "market:etf-flows:current:v1"],
  minTier: 0,
  variants: "*",
});
