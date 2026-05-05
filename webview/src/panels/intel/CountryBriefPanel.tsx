import {
  useEffect,
  useMemo,
  useState,
  type ReactElement,
} from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadCountryBrief,
  type CountryBriefOutcome,
  type CountryBriefQuery,
  type CountryBriefResponse,
} from "../../data/loaders/intel/country-brief";
import { IntelEntityChip, registerPanel } from "./index";

/** Stable id for `usePanelStore` registration. */
export const COUNTRY_BRIEF_PANEL_ID = "intel/country-brief";

/** Tier gate. */
export const REQUIRED_TIER = 0;

/** Default country shown on first mount. */
export const DEFAULT_COUNTRY = "Iran";

export interface CountryBriefPanelProps {
  /** Initial / pre-set country. */
  country?: string;
  /** Override the loader (testing). */
  load?: typeof loadCountryBrief;
  /** Wall-clock ms used as "now" for the relative timestamp. */
  now?: number;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: CountryBriefOutcome };

/**
 * Country brief panel — M3 family 4.1, T4.1.8.
 *
 * Compact summary card variant of the
 * `CountryDeepDivePanel`: one sentence-length headline, a tight
 * counts strip, the top-1 actor, and the freshest article.
 * Designed to fit in a sidebar or a small grid cell so a
 * dashboard can surface multiple countries at once without each
 * one taking the wide deep-dive footprint.
 */
export function CountryBriefPanel(
  props: CountryBriefPanelProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(COUNTRY_BRIEF_PANEL_ID));

  const [country, setCountry] = useState<string>(props.country ?? DEFAULT_COUNTRY);
  const [view, setView] = useState<ViewState>({ kind: "loading" });

  const effectiveQuery = useMemo<CountryBriefQuery>(
    () => ({ country }),
    [country],
  );

  useEffect(() => {
    setLayout(COUNTRY_BRIEF_PANEL_ID, { rowSpan: 1, colSpan: 1 });
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
    const loader = props.load ?? loadCountryBrief;
    void loader(effectiveQuery).then((outcome) => {
      if (cancelled) return;
      setView({ kind: "ready", outcome });
    });
    return () => {
      cancelled = true;
    };
  }, [hasTier, effectiveQuery, props.load]);

  if (isHidden) return <></>;

  return (
    <section
      data-panel-id={COUNTRY_BRIEF_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-2"
      aria-label="Country brief"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Country brief</h3>
        <CountryPicker value={country} onChange={setCountry} />
      </header>
      {renderBody(view, props.now ?? Date.now())}
    </section>
  );
}

function renderBody(view: ViewState, now: number): ReactElement {
  if (view.kind === "loading") {
    return (
      <div
        role="status"
        aria-live="polite"
        className="text-xs text-[var(--pellucid-muted)]"
      >
        Loading brief…
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
  if (out.kind === "ready") {
    const r = out.response;
    return (
      <div data-state="ready" className="flex flex-col gap-2">
        <div
          data-component="BriefHeadline"
          className="flex items-baseline justify-between gap-2"
        >
          <h4 className="text-base font-semibold">{r.country}</h4>
          <IntelEntityChip
            entity={{
              id: `region:${r.region}`,
              kind: "region",
              name: r.region,
            }}
          />
        </div>
        <p
          data-field="summary"
          className="text-xs leading-snug text-[var(--pellucid-fg)]"
        >
          {r.summary}
        </p>
        <CountsStrip totals={r.totals} />
        {r.topActor ? (
          <div
            data-component="BriefTopActor"
            data-actor={r.topActor.name}
            className="flex items-center justify-between text-[11px]"
          >
            <span className="text-[var(--pellucid-muted)]">Top actor</span>
            <div className="flex items-center gap-1">
              <IntelEntityChip
                entity={{
                  id: `actor:${r.topActor.name}`,
                  kind: "actor",
                  name: r.topActor.name,
                }}
              />
              <span className="font-mono text-[var(--pellucid-muted)]">
                {r.topActor.events}E
              </span>
            </div>
          </div>
        ) : null}
        {r.topArticle ? (
          <a
            data-component="BriefTopArticle"
            data-article-url={r.topArticle.url}
            href={r.topArticle.url}
            target="_blank"
            rel="noreferrer"
            className="block rounded border border-[var(--pellucid-border)] bg-[var(--pellucid-surface-raised)] p-2 text-[11px] hover:border-[var(--pellucid-info)]"
          >
            <span
              data-field="article-title"
              className="block font-semibold text-[var(--pellucid-fg)] line-clamp-2"
            >
              {r.topArticle.title}
            </span>
            <span
              data-field="article-meta"
              className="mt-0.5 block font-mono text-[10px] text-[var(--pellucid-muted)]"
            >
              {r.topArticle.domain} · {formatRelative(parseGdeltSeenDate(r.topArticle.seenDate) ?? now, now)}
            </span>
          </a>
        ) : null}
        <footer
          data-component="BriefFooter"
          className="flex items-center justify-between text-[10px] text-[var(--pellucid-muted)]"
        >
          <span data-field="assembled-at">
            {formatAssembledAt(r.assembledAtMs)}
          </span>
          {r.stale ? (
            <span data-field="stale" aria-live="polite">
              cached
            </span>
          ) : null}
        </footer>
      </div>
    );
  }
  if (out.code === "bootstrap_upstream_empty") {
    return (
      <div
        role="alert"
        data-state="outage"
        className="text-xs text-[var(--pellucid-warn)]"
      >
        Brief offline — retry shortly.
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

function CountsStrip({
  totals,
}: {
  totals: CountryBriefResponse["totals"];
}): ReactElement {
  return (
    <div
      data-component="BriefCounts"
      className="flex items-center gap-2 text-[10px] font-mono text-[var(--pellucid-muted)]"
    >
      <span data-field="counts-events">{totals.events}E</span>
      <span data-field="counts-incidents">{totals.incidents}I</span>
      <span data-field="counts-messages">{totals.messages}M</span>
    </div>
  );
}

function CountryPicker({
  value,
  onChange,
}: {
  value: string;
  onChange: (next: string) => void;
}): ReactElement {
  return (
    <label
      className="flex items-center gap-1 text-[10px] uppercase font-mono text-[var(--pellucid-muted)]"
      data-field="country-picker"
    >
      <span>Country</span>
      <input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        aria-label="Country"
        className="rounded border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] px-2 py-0.5 text-[11px] text-[var(--pellucid-fg)] focus:outline-none focus:border-[var(--pellucid-info)]"
      />
    </label>
  );
}

function labelForCode(code: string): string {
  switch (code) {
    case "invalid_request":
      return "Invalid request";
    case "cache_failure":
      return "Cache failure";
    case "cache_shape":
      return "Wire-shape mismatch";
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

/** Parse a GDELT `YYYYMMDDTHHMMSSZ` timestamp into wall-clock ms.
 *  Pure, exported for unit tests. */
export function parseGdeltSeenDate(seenDate: string): number | null {
  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(seenDate);
  if (!m) return null;
  const [, y, mo, d, h, mi, s] = m;
  const ms = Date.UTC(
    Number(y),
    Number(mo) - 1,
    Number(d),
    Number(h),
    Number(mi),
    Number(s),
  );
  return Number.isFinite(ms) ? ms : null;
}

/** Compact relative formatter (`8m ago` / `2h ago` / `3d ago`).
 *  Pure, exported for unit tests. */
export function formatRelative(timestampMs: number, nowMs: number): string {
  const deltaMs = nowMs - timestampMs;
  if (deltaMs < 0) return "just now";
  const sec = Math.floor(deltaMs / 1000);
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  return `${day}d ago`;
}

/** HH:MM:SS UTC formatter shared with the other intel panels. */
export function formatAssembledAt(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const d = new Date(ms);
  const hh = String(d.getUTCHours()).padStart(2, "0");
  const mm = String(d.getUTCMinutes()).padStart(2, "0");
  const ss = String(d.getUTCSeconds()).padStart(2, "0");
  return `${hh}:${mm}:${ss} UTC`;
}

// Register in the family registry on import.
registerPanel({
  id: COUNTRY_BRIEF_PANEL_ID,
  title: "Country brief",
  blurb: "Compact one-line summary of country signal across feeds.",
  component: CountryBriefPanel as React.ComponentType<unknown>,
  cacheKeys: [
    "conflict:events-24h:v1",
    "conflict:incident-feed:v1",
    "telegram:recent-feed:v1",
  ],
  minTier: REQUIRED_TIER,
  variants: "*",
});
