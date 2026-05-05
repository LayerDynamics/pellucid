import {
  useEffect,
  useMemo,
  useState,
  type ReactElement,
} from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadGdeltFeed,
  parseGdeltSeenDate,
  type GdeltArticle,
  type GdeltFeedOutcome,
  type GdeltFeedQuery,
} from "../../data/loaders/intel/gdelt";
import { IntelEntityChip, NewsCard, registerPanel } from "./index";

/** Stable id for `usePanelStore` registration. */
export const GDELT_INTEL_PANEL_ID = "intel/gdelt";

/** Tier gate. The handler is anonymous (FAST-tier cache reader,
 *  no upstream side-effects) so the panel ships at tier 0. */
export const REQUIRED_TIER = 0;

/** Default page size — matches the handler's `DEFAULT_LIMIT`. */
export const DEFAULT_LIMIT = 50;

export interface GdeltIntelPanelProps {
  /** Optional pre-filter passed to the loader. */
  query?: GdeltFeedQuery;
  /** Override the loader (testing). */
  load?: typeof loadGdeltFeed;
  /** Wall-clock ms used as "now" by the relative-time formatter.
   *  Defaults to `Date.now()`; tests override for determinism. */
  now?: number;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: GdeltFeedOutcome };

/**
 * GDELT intelligence panel — M3 family 4.1, T4.1.4.
 *
 * Renders the FAST-tier snapshot the `seed_gdelt_intel` seeder
 * publishes: a list of news articles GDELT has indexed inside
 * the last 24h matching the global incident-theme query
 * (`KILL OR WOUND OR ARMEDCONFLICT OR TERROR`).
 *
 * Each row renders as a [`NewsCard`] (shared with the news
 * family) — GDELT carries no severity tag, so the card omits the
 * severity badge. The panel adds a country-filter input bound
 * to the loader's `?country=` query knob.
 *
 * View states:
 *  - **loading** — request in flight.
 *  - **locked** — caller's effective tier < `REQUIRED_TIER`.
 *  - **ready (success)** — paginated card list with a
 *    country-filter chrome + an "as of" footer that surfaces
 *    the seeder's `assembledAtMs`.
 *  - **ready (outage)** — 503 path with Retry-After countdown.
 *  - **ready (error)** — generic failure envelope.
 */
export function GdeltIntelPanel(
  props: GdeltIntelPanelProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(GDELT_INTEL_PANEL_ID));

  const [view, setView] = useState<ViewState>({ kind: "loading" });
  const [country, setCountry] = useState<string>(props.query?.country ?? "");

  const effectiveQuery = useMemo<GdeltFeedQuery>(() => {
    const q: GdeltFeedQuery = {
      limit: props.query?.limit ?? DEFAULT_LIMIT,
    };
    if (country.trim().length > 0) q.country = country.trim();
    return q;
  }, [props.query?.limit, country]);

  useEffect(() => {
    setLayout(GDELT_INTEL_PANEL_ID, { rowSpan: 2, colSpan: 2 });
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
    const loader = props.load ?? loadGdeltFeed;
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
      data-panel-id={GDELT_INTEL_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="GDELT intelligence feed"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">GDELT intel</h3>
        <CountryFilter
          value={country}
          onChange={setCountry}
        />
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
        Loading GDELT intel…
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
    if (out.response.rows.length === 0) {
      return (
        <div
          data-state="empty"
          role="status"
          className="text-xs text-[var(--pellucid-muted)]"
        >
          No incidents match the current filters.
        </div>
      );
    }
    return (
      <div data-state="ready" className="flex flex-col gap-2">
        <ul
          className="flex flex-col gap-2"
          aria-label={`${out.response.rows.length} GDELT articles`}
        >
          {out.response.rows.map((row) => (
            <li key={row.url}>
              <GdeltRow article={row} now={now} />
            </li>
          ))}
        </ul>
        <footer
          data-component="GdeltIntelFooter"
          className="flex items-center justify-between text-[11px] text-[var(--pellucid-muted)]"
        >
          <span>
            Showing {out.response.rows.length} of {out.response.total}
          </span>
          <span data-field="assembled-at">
            Snapshot: {formatAssembledAt(out.response.assembledAtMs)}
          </span>
          {out.response.stale ? (
            <span data-field="stale" aria-live="polite">
              Showing cached snapshot.
            </span>
          ) : null}
        </footer>
      </div>
    );
  }
  // out.kind === "error"
  if (out.code === "bootstrap_upstream_empty") {
    return (
      <div
        role="alert"
        data-state="outage"
        className="text-xs text-[var(--pellucid-warn)]"
      >
        <strong>GDELT pipeline offline.</strong>
        <br />
        {out.retryAfterSecs ? (
          <>Retry available in {out.retryAfterSecs}s.</>
        ) : (
          <>Retry shortly.</>
        )}
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

interface GdeltRowProps {
  article: GdeltArticle;
  now: number;
}

function GdeltRow({ article, now }: GdeltRowProps): ReactElement {
  const publishedAtMs = parseGdeltSeenDate(article.seenDate) ?? now;
  return (
    <div
      data-component="GdeltRow"
      data-news-url={article.url}
      className="flex flex-col gap-1"
    >
      <NewsCard
        item={{
          id: article.url,
          title: article.title,
          source: article.domain,
          publishedAtMs,
          url: article.url,
        }}
        now={now}
      />
      <div
        className="flex flex-wrap items-center gap-1"
        data-field="entity-chips"
      >
        {article.sourceCountry ? (
          <IntelEntityChip
            entity={{
              id: `country:${article.sourceCountry}`,
              kind: "country",
              name: article.sourceCountry,
            }}
          />
        ) : null}
        {article.language ? (
          <IntelEntityChip
            entity={{
              id: `topic:${article.language}`,
              kind: "topic",
              name: article.language,
            }}
          />
        ) : null}
      </div>
    </div>
  );
}

function CountryFilter({
  value,
  onChange,
}: {
  value: string;
  onChange: (next: string) => void;
}): ReactElement {
  return (
    <label
      className="flex items-center gap-1 text-[11px] uppercase font-mono text-[var(--pellucid-muted)]"
      data-field="country-filter"
    >
      <span>Country</span>
      <input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="any"
        aria-label="Country filter"
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

/**
 * Format the seeder's `assembledAtMs` as a short UTC string
 * ("12:00:00 UTC"). Pure — exported for the unit test that pins
 * the format.
 */
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
  id: GDELT_INTEL_PANEL_ID,
  title: "GDELT intel",
  blurb: "FAST-tier snapshot of GDELT incident-theme news.",
  component: GdeltIntelPanel as React.ComponentType<unknown>,
  cacheKeys: ["conflict:incident-feed:v1"],
  minTier: REQUIRED_TIER,
  variants: "*",
});
