import { useEffect, useRef, useState, type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import { useNewsStore } from "../../state/useNewsStore";
import {
  subscribe as defaultSubscribe,
  type LiveError,
  type LiveSubscription,
  type SubscribeQuery,
} from "../../data/loaders/news/live";
import type { NewsArticle } from "../../data/loaders/news/list";
import { NewsCard } from "./NewsCard";
import {
  SignalSeverityBadge,
  type SignalSeverity,
} from "./SignalSeverityBadge";
import { registerPanel } from "./index";

/** Stable id for `usePanelStore` registration. */
export const LIVE_NEWS_PANEL_ID = "news/live";

/** Minimum tier required to render. The plan reserves the live
 *  endpoint for tier 1+ — anonymous users get the static feed
 *  via `NewsPanel` only, since SSE connections are stateful and
 *  expensive to keep open at the gateway. */
export const REQUIRED_TIER = 1;

/** Hard cap on how many articles the panel keeps in its rolling
 *  list. New articles arriving past this cap evict the oldest;
 *  prevents unbounded memory growth on a long-lived connection. */
export const MAX_BUFFER = 200;

export interface LiveNewsPanelProps {
  /** Optional pre-filter passed to the subscribe call. */
  query?: SubscribeQuery;
  /** Override the subscribe function (testing). */
  subscribe?: typeof defaultSubscribe;
  /** Wall-clock ms used as "now" by `<NewsCard>`. Defaults to
   *  `Date.now()`; tests override for determinism. */
  now?: number;
}

type ConnectionState =
  | { kind: "connecting" }
  | { kind: "open" }
  | { kind: "outage"; reason: string }
  | { kind: "error"; error: LiveError }
  | { kind: "disconnected" };

const SEVERITY_CHOICES: SignalSeverity[] = [
  "info",
  "warn",
  "high",
  "critical",
];

/**
 * Live news panel — M3 family 4.1, T4.1.2.
 *
 * Maintains an EventSource subscription to
 * `/api/news/v1/list-live`. The connection delivers an initial
 * `ready` snapshot followed by per-article deltas as the seeder
 * pipeline publishes them. The panel:
 *
 *  - prepends each new article into a rolling list (capped at
 *    [`MAX_BUFFER`]),
 *  - mirrors the list into [`useNewsStore.feed`] so sibling
 *    panels (`NewsPanel`, `BreakingNewsBanner`) see the same
 *    canonical ordering,
 *  - renders a connection-state header (connecting / open /
 *    outage / error / disconnected) so operators can see
 *    transport health at a glance,
 *  - filters by severity client-side via the same toolbar
 *    `NewsPanel` exposes (and re-subscribes when the floor
 *    changes so the server-side filter trims at the source too).
 */
export function LiveNewsPanel(props: LiveNewsPanelProps): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(LIVE_NEWS_PANEL_ID));
  const setFeed = useNewsStore((s) => s.setFeed);

  const [connection, setConnection] = useState<ConnectionState>({
    kind: "connecting",
  });
  const [articles, setArticles] = useState<NewsArticle[]>([]);
  const [severityFloor, setSeverityFloor] = useState<SignalSeverity | null>(
    props.query?.severity ?? null,
  );

  // Track the latest articles list in a ref so the EventSource
  // callbacks (which capture the initial render's closure) can
  // append to the freshest snapshot rather than overwrite the
  // entire array each delta.
  const articlesRef = useRef<NewsArticle[]>([]);
  articlesRef.current = articles;

  useEffect(() => {
    setLayout(LIVE_NEWS_PANEL_ID, { rowSpan: 2, colSpan: 2 });
  }, [setLayout]);

  useEffect(() => {
    if (!hasTier(REQUIRED_TIER)) {
      setConnection({ kind: "disconnected" });
      return;
    }
    setConnection({ kind: "connecting" });
    setArticles([]);
    articlesRef.current = [];

    const sub: LiveSubscription = (props.subscribe ?? defaultSubscribe)(
      {
        onReady: (snap) => {
          articlesRef.current = snap.articles.slice(0, MAX_BUFFER);
          setArticles(articlesRef.current);
          setFeed(articlesRef.current);
          setConnection({ kind: "open" });
        },
        onArticle: (a) => {
          // Dedupe by id + prepend (latest first) + cap.
          const existing = articlesRef.current.filter((x) => x.id !== a.id);
          const next = [a, ...existing].slice(0, MAX_BUFFER);
          articlesRef.current = next;
          setArticles(next);
          setFeed(next);
        },
        onOutage: (reason) => {
          setConnection({ kind: "outage", reason });
        },
        onError: (err) => {
          setConnection({ kind: "error", error: err });
        },
        onConnectionError: () => {
          // Native EventSource auto-reconnects; surface the
          // transient state in the header without dropping the
          // accumulated article list.
          setConnection({ kind: "connecting" });
        },
      },
      { ...(props.query ?? {}), ...(severityFloor ? { severity: severityFloor } : {}) },
    );
    return () => sub.close();
  }, [hasTier, props.subscribe, props.query, severityFloor, setFeed]);

  if (isHidden) return <></>;

  const now = props.now ?? Date.now();

  return (
    <section
      data-panel-id={LIVE_NEWS_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Live news"
    >
      <header className="flex items-baseline justify-between gap-2">
        <div className="flex items-baseline gap-2">
          <h3 className="text-sm font-semibold">Live news</h3>
          <ConnectionDot state={connection} />
        </div>
        <SeverityFloorChips
          current={severityFloor}
          onChange={setSeverityFloor}
        />
      </header>
      {renderBody(connection, articles, now)}
    </section>
  );
}

function renderBody(
  connection: ConnectionState,
  articles: NewsArticle[],
  now: number,
): ReactElement {
  if (connection.kind === "disconnected") {
    return (
      <div
        role="alert"
        data-state="locked"
        className="text-xs text-[var(--pellucid-warn)]"
      >
        Locked — live stream requires tier {REQUIRED_TIER} or higher.
      </div>
    );
  }
  if (connection.kind === "outage") {
    return (
      <div
        role="alert"
        data-state="outage"
        data-outage-reason={connection.reason}
        className="text-xs text-[var(--pellucid-warn)]"
      >
        News pipeline offline — waiting for the seeder to recover.
      </div>
    );
  }
  if (connection.kind === "error") {
    return (
      <div
        role="alert"
        data-state="error"
        data-error-code={connection.error.code}
        className="text-xs text-[var(--pellucid-danger)]"
      >
        <strong>Stream error</strong>
        <br />
        {connection.error.message}
      </div>
    );
  }
  if (articles.length === 0) {
    return (
      <div
        data-state="waiting"
        role="status"
        aria-live="polite"
        className="text-xs text-[var(--pellucid-muted)]"
      >
        {connection.kind === "connecting"
          ? "Connecting…"
          : "Waiting for the next article…"}
      </div>
    );
  }
  return (
    <div data-state="streaming" className="flex flex-col gap-2">
      <ul
        className="flex flex-col gap-2"
        aria-label={`${articles.length} live articles`}
      >
        {articles.map((a) => (
          <li key={a.id}>
            <NewsCard item={a} now={now} />
          </li>
        ))}
      </ul>
      <footer className="text-[11px] text-[var(--pellucid-muted)]">
        Buffer: {articles.length} / {MAX_BUFFER}
      </footer>
    </div>
  );
}

function ConnectionDot({ state }: { state: ConnectionState }): ReactElement {
  const statusLabel: Record<ConnectionState["kind"], string> = {
    connecting: "Connecting",
    open: "Live",
    outage: "Outage",
    error: "Error",
    disconnected: "Disconnected",
  };
  const colorClass: Record<ConnectionState["kind"], string> = {
    connecting: "bg-[var(--pellucid-muted)]",
    open: "bg-[var(--pellucid-info)]",
    outage: "bg-[var(--pellucid-warn)]",
    error: "bg-[var(--pellucid-danger)]",
    disconnected: "bg-[var(--pellucid-muted)]",
  };
  return (
    <span
      data-component="ConnectionDot"
      data-connection-state={state.kind}
      role="status"
      aria-live="polite"
      aria-label={`Connection: ${statusLabel[state.kind]}`}
      className="inline-flex items-center gap-1 text-[10px] uppercase font-mono tracking-wide text-[var(--pellucid-muted)]"
    >
      <span
        className={`inline-block h-1.5 w-1.5 rounded-full ${colorClass[state.kind]}`}
        aria-hidden="true"
      />
      {statusLabel[state.kind]}
    </span>
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

// Register in the family registry on import.
registerPanel({
  id: LIVE_NEWS_PANEL_ID,
  title: "Live news",
  blurb: "Real-time stream of new headlines as they arrive.",
  component: LiveNewsPanel as React.ComponentType<unknown>,
  cacheKeys: ["news:articles:list:v1"],
  minTier: REQUIRED_TIER,
  variants: "*",
});
