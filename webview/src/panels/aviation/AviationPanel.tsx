import { useEffect, useState, type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadFlightStatus,
  type FlightStatusOutcome,
  type FlightStatusQuery,
} from "../../data/loaders/aviation";

/** Stable id for usePanelStore registration. Mirrors the original
 *  WorldMonitor panel id so layout migrations carry over. */
export const AVIATION_PANEL_ID = "aviation/flight-status";

/** Minimum tier required to render. The aviation handler is
 *  Anonymous in M0, but the panel scaffolding pins **tier 1**
 *  (Free, signed-in) so the tier-gating UX path — including the
 *  locked-state branch — is exercised end-to-end at M1. */
export const REQUIRED_TIER = 1;

export interface AviationPanelProps {
  /** Query the panel should resolve. Real layouts feed this from
   *  a search box or URL state; the M1 demo passes a fixed
   *  example. */
  query: FlightStatusQuery;
  /** Override the loader (testing). */
  load?: typeof loadFlightStatus;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: FlightStatusOutcome };

/**
 * Aviation flight-status panel — M1 end-to-end demonstration.
 *
 * Renders the four states the plan calls out:
 *  - **loading** — request in flight.
 *  - **locked** — caller's effective tier < `REQUIRED_TIER`.
 *  - **ready (success)** — `<dl>` with the flight metadata.
 *  - **ready (error)** — error envelope rendered with the
 *    handler-side code so the user can branch (outage banner,
 *    upgrade prompt, validation message).
 *
 * The panel registers itself in `usePanelStore` on mount so the
 * grid layout system tracks it.
 */
export function AviationPanel(props: AviationPanelProps): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(AVIATION_PANEL_ID));
  const [view, setView] = useState<ViewState>({ kind: "loading" });

  useEffect(() => {
    setLayout(AVIATION_PANEL_ID, { rowSpan: 1, colSpan: 2 });
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
    const loader = props.load ?? loadFlightStatus;
    void loader(props.query).then((outcome) => {
      if (cancelled) return;
      setView({ kind: "ready", outcome });
    });
    return () => {
      cancelled = true;
    };
  }, [hasTier, props.query, props.load]);

  if (isHidden) return <></>;

  return (
    <section
      data-panel-id={AVIATION_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3"
      aria-label="Aviation flight status"
    >
      <header className="mb-2 flex items-baseline justify-between">
        <h3 className="text-sm font-semibold">Flight status</h3>
        <span className="text-xs text-[var(--pellucid-muted)]">
          {props.query.flight} · {props.query.date} · {props.query.origin}
        </span>
      </header>
      {renderBody(view)}
    </section>
  );
}

function renderBody(view: ViewState): ReactElement {
  if (view.kind === "loading") {
    return (
      <div role="status" aria-live="polite" className="text-xs text-[var(--pellucid-muted)]">
        Loading flight status…
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
  // view.kind === "ready"
  const out = view.outcome;
  if (out.kind === "ready") {
    const s = out.status;
    return (
      <dl
        data-state="ready"
        className="grid grid-cols-[max-content_1fr] gap-x-3 gap-y-1 text-xs"
      >
        <Field label="Status" value={s.status} />
        <Field label="From" value={s.origin} />
        <Field label="To" value={s.destination} />
        <Field label="Sched dep." value={s.scheduled_departure} />
        <Field label="Sched arr." value={s.scheduled_arrival} />
        {s.departure_gate ? (
          <Field label="Gate (dep)" value={s.departure_gate} />
        ) : null}
        {s.arrival_gate ? <Field label="Gate (arr)" value={s.arrival_gate} /> : null}
      </dl>
    );
  }
  // out.kind === "error"
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
      {out.retryAfterSecs ? (
        <>
          <br />
          <span>Retry in {out.retryAfterSecs}s.</span>
        </>
      ) : null}
    </div>
  );
}

function Field({ label, value }: { label: string; value: string }): ReactElement {
  return (
    <>
      <dt className="text-[var(--pellucid-muted)]">{label}</dt>
      <dd>{value}</dd>
    </>
  );
}

function labelForCode(code: string): string {
  switch (code) {
    case "invalid_request":
      return "Invalid request";
    case "upstream_failure":
      return "Upstream unreachable";
    case "cache_failure":
      return "Cache failure";
    case "entitlement_forbidden":
      return "Premium tier required";
    case "clerk_unauthorized":
      return "Sign-in required";
    case "network":
      return "Network error";
    default:
      return "Error";
  }
}
