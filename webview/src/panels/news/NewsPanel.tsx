import { useEffect, useMemo, useState, type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import { useNewsStore } from "../../state/useNewsStore";
import {
  loadNewsArticles,
  type ListArticlesOutcome,
  type ListArticlesQuery,
} from "../../data/loaders/news/list";
import { NewsCard } from "./NewsCard";
import {
  SignalSeverityBadge,
  type SignalSeverity,
} from "./SignalSeverityBadge";
import { registerPanel } from "./index";

/** Stable id for `usePanelStore` registration. Mirrors the
 *  registry descriptor at `panels/news/index.ts`. */
export const NEWS_PANEL_ID = "news/feed";

/** Minimum tier required to render. The `news/v1/list-articles`
 *  handler is anonymous, so the panel ships at tier 0 (Anonymous). */
export const REQUIRED_TIER = 0;

/** Default page size — matches the handler's `DEFAULT_LIMIT`. */
export const DEFAULT_LIMIT = 50;

export interface NewsPanelProps {
  /** Optional pre-filter passed to the loader. Real layouts can
   *  feed this from a search box or URL state; the M3 demo passes
   *  an empty query. */
  query?: ListArticlesQuery;
  /** Override the loader (testing). */
  load?: typeof loadNewsArticles;
  /** Wall-clock ms used as "now" by `<NewsCard>`. Defaults to
   *  `Date.now()`; tests override for determinism. */
  now?: number;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: ListArticlesOutcome };

const SEVERITY_CHOICES: SignalSeverity[] = [
  "info",
  "warn",
  "high",
  "critical",
];

/**
 * News panel — M3 family 4.1 first panel (T4.1.1).
 *
 * Renders the same five view states the M3 spec calls out:
 *  - **loading** — request in flight.
 *  - **locked** — caller's effective tier < `REQUIRED_TIER`.
 *  - **ready (success)** — paginated `<NewsCard>` list with
 *    severity-floor + limit chrome.
 *  - **ready (outage)** — 503 path with Retry-After countdown.
 *  - **ready (error)** — generic failure envelope.
 *
 * The panel mirrors articles into `useNewsStore.feed` on success
 * so other family panels (`BreakingNewsBanner`, `LiveNewsPanel`)
 * see the same canonical list without re-fetching.
 */
export function NewsPanel(props: NewsPanelProps): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(NEWS_PANEL_ID));
  const setFeed = useNewsStore((s) => s.setFeed);

  const [view, setView] = useState<ViewState>({ kind: "loading" });
  const [severityFloor, setSeverityFloor] = useState<SignalSeverity | null>(
    props.query?.severity ?? null,
  );

  // Stable query — re-built whenever the user toggles the severity
  // floor so `useEffect` triggers a refetch.
  const effectiveQuery = useMemo<ListArticlesQuery>(() => {
    const q: ListArticlesQuery = { limit: props.query?.limit ?? DEFAULT_LIMIT };
    if (severityFloor) q.severity = severityFloor;
    return q;
  }, [props.query?.limit, severityFloor]);

  useEffect(() => {
    setLayout(NEWS_PANEL_ID, { rowSpan: 2, colSpan: 2 });
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
    const loader = props.load ?? loadNewsArticles;
    void loader(effectiveQuery).then((outcome) => {
      if (cancelled) return;
      setView({ kind: "ready", outcome });
      if (outcome.kind === "ready") {
        setFeed(outcome.response.articles);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [hasTier, effectiveQuery, props.load, setFeed]);

  if (isHidden) return <></>;

  return (
    <section
      data-panel-id={NEWS_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="News feed"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">News</h3>
        <SeverityFloorChips
          current={severityFloor}
          onChange={setSeverityFloor}
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
        Loading news…
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
  // ready
  const out = view.outcome;
  if (out.kind === "ready") {
    if (out.response.articles.length === 0) {
      return (
        <div
          data-state="empty"
          role="status"
          className="text-xs text-[var(--pellucid-muted)]"
        >
          No articles match the current filters.
        </div>
      );
    }
    return (
      <div data-state="ready" className="flex flex-col gap-2">
        <ul
          className="flex flex-col gap-2"
          aria-label={`${out.response.articles.length} news articles`}
        >
          {out.response.articles.map((a) => (
            <li key={a.id}>
              <NewsCard item={a} now={now} />
            </li>
          ))}
        </ul>
        <footer className="flex items-center justify-between text-[11px] text-[var(--pellucid-muted)]">
          <span>
            Showing {out.response.articles.length} of {out.response.total}
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
        <strong>News pipeline offline.</strong>
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

function SeverityFloorChips({
  current,
  onChange,
}: {
  current: SignalSeverity | null;
  onChange: (next: SignalSeverity | null) => void;
}): ReactElement {
  return (
    <div
      role="toolbar"
      aria-label="Severity floor"
      className="flex items-center gap-1"
    >
      <button
        type="button"
        data-severity-floor="all"
        aria-pressed={current === null ? "true" : "false"}
        onClick={() => onChange(null)}
        className="rounded-full border border-[var(--pellucid-border)] px-2 py-0.5 text-[11px] uppercase font-mono"
      >
        All
      </button>
      {SEVERITY_CHOICES.map((sev) => (
        <button
          key={sev}
          type="button"
          data-severity-floor={sev}
          aria-pressed={current === sev ? "true" : "false"}
          onClick={() => onChange(current === sev ? null : sev)}
          className="leading-none"
        >
          <SignalSeverityBadge severity={sev} compact />
        </button>
      ))}
    </div>
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

// Register the panel in the family registry on import. Mirrors
// the entry that was reserved for this id in `index.ts`.
registerPanel({
  id: NEWS_PANEL_ID,
  title: "News",
  blurb: "Curated headlines across every monitored source.",
  component: NewsPanel as React.ComponentType<unknown>,
  cacheKeys: ["news:articles:list:v1"],
  minTier: REQUIRED_TIER,
  variants: "*",
});
