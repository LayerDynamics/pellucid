import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactElement,
} from "react";

import {
  Toast,
  ToastAction,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
  type ToastTone,
} from "../../components/primitives/Toast";
import { useAuthStore } from "../../state/useAuthStore";
import { useNewsStore } from "../../state/useNewsStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadBreakingNews,
  type GetBreakingOutcome,
  type GetBreakingQuery,
} from "../../data/loaders/news/breaking";
import type { NewsArticle } from "../../data/loaders/news/list";
import type { SignalSeverity } from "./SignalSeverityBadge";
import { registerPanel } from "./index";

/** Stable id for `usePanelStore` registration. */
export const BREAKING_NEWS_PANEL_ID = "news/breaking";

/** Tier gate. The handler is anonymous (same `news:articles:list:v1`
 *  reader as `NewsPanel`), so the banner ships at tier 0. */
export const REQUIRED_TIER = 0;

/** How often the banner re-polls the loader. 30 s matches the
 *  seeder's atomic_publish cadence + Litestream replica window so
 *  a freshly published article surfaces inside one poll. */
export const POLL_INTERVAL_MS = 30_000;

/** How long each Radix Toast stays open before auto-dismissing.
 *  Long enough to read a headline + click through, short enough
 *  that a queue of breaking items doesn't pile up forever. */
export const AUTO_DISMISS_MS = 8_000;

/** Hard cap on how many toast records the banner keeps alive at
 *  once. Older toasts are evicted as new breaking items arrive
 *  so a stalled `onOpenChange` callback can't leak forever. */
export const MAX_OPEN_TOASTS = 5;

export interface BreakingNewsBannerProps {
  /** Optional pre-filter passed to the loader. Real layouts may
   *  feed this from a variant or user setting; the M3 demo
   *  passes an empty query. */
  query?: GetBreakingQuery;
  /** Override the loader (testing). */
  load?: typeof loadBreakingNews;
  /** Override the polling interval. Tests use a tiny value to
   *  drive the loop without real time. */
  pollIntervalMs?: number;
  /** Override the auto-dismiss duration handed to Radix Toast. */
  autoDismissMs?: number;
  /** Optional callback fired the moment a Radix toast opens.
   *  Tests use this to assert one-toast-per-new-article without
   *  reaching into Radix internals. */
  onToastOpen?: (article: NewsArticle) => void;
  /** Optional callback fired when the user clicks the toast's
   *  click-through anchor. The browser still navigates; this
   *  callback only mirrors the event for analytics + tests. */
  onToastClick?: (article: NewsArticle) => void;
}

/**
 * One live toast — bookkeeping plus the article payload. The
 * banner keeps an array of these in state so the JSX can render
 * one Radix `<Toast>` per active record.
 */
interface ToastRecord {
  /** Stable key for the React list. The article id is reused as
   *  the key — duplicate breaking ids are filtered out before a
   *  record is ever created. */
  key: string;
  article: NewsArticle;
}

/**
 * Breaking-news banner — M3 family 4.1, T4.1.3.
 *
 * Polls `/api/news/v1/get-breaking` every [`POLL_INTERVAL_MS`].
 * For every article id the banner has not toasted before, opens a
 * Radix Toast with the headline, source, and a click-through
 * anchor. The toast auto-dismisses after [`AUTO_DISMISS_MS`].
 *
 * The component renders nothing visible until the first breaking
 * article arrives — the `<ToastViewport>` is positioned by the
 * shared design-token wrapper at `components/primitives/Toast`.
 *
 * Mirrors the freshest article into [`useNewsStore.setBreaking`]
 * so sibling panels (`NewsPanel`, `LiveNewsPanel`) and the global
 * status chrome can react without re-fetching.
 */
export function BreakingNewsBanner(
  props: BreakingNewsBannerProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(BREAKING_NEWS_PANEL_ID));
  const setBreaking = useNewsStore((s) => s.setBreaking);

  const pollIntervalMs = props.pollIntervalMs ?? POLL_INTERVAL_MS;
  const autoDismissMs = props.autoDismissMs ?? AUTO_DISMISS_MS;
  const loader = props.load ?? loadBreakingNews;

  const [toasts, setToasts] = useState<ToastRecord[]>([]);
  const [lastError, setLastError] = useState<{
    code: string;
    message: string;
  } | null>(null);

  // Refs for the loop's bookkeeping that must not trigger
  // re-renders. `seenIdsRef` dedupes across polls; `sinceMsRef`
  // gives the loader a server-side cutoff so the wire response
  // shrinks once the banner is past the warm-up tick.
  const seenIdsRef = useRef<Set<string>>(new Set());
  const sinceMsRef = useRef<number | undefined>(undefined);
  const onToastOpenRef = useRef(props.onToastOpen);
  onToastOpenRef.current = props.onToastOpen;

  useEffect(() => {
    setLayout(BREAKING_NEWS_PANEL_ID, { rowSpan: 1, colSpan: 4 });
  }, [setLayout]);

  // Reset the dedupe set + cutoff every time the query knobs
  // change so a severity-floor toggle re-toasts the matching
  // articles instead of swallowing them as "already seen".
  useEffect(() => {
    seenIdsRef.current = new Set();
    sinceMsRef.current = undefined;
  }, [props.query?.severity, props.query?.limit]);

  useEffect(() => {
    if (!hasTier(REQUIRED_TIER)) return;

    let cancelled = false;

    const poll = async () => {
      const q: GetBreakingQuery = { ...(props.query ?? {}) };
      if (typeof sinceMsRef.current === "number") {
        q.sinceMs = sinceMsRef.current;
      }
      const outcome: GetBreakingOutcome = await loader(q);
      if (cancelled) return;
      if (outcome.kind === "error") {
        setLastError({ code: outcome.code, message: outcome.message });
        return;
      }
      setLastError(null);
      const fresh = outcome.response.articles.filter(
        (a) => !seenIdsRef.current.has(a.id),
      );
      if (fresh.length === 0) {
        // Still update the cutoff against the freshest visible
        // article so subsequent polls return less wire data.
        const newest = outcome.response.articles[0];
        if (newest && typeof newest.publishedAtMs === "number") {
          sinceMsRef.current = Math.max(
            sinceMsRef.current ?? 0,
            newest.publishedAtMs,
          );
        }
        return;
      }
      // Mark every fresh article as seen + bump the cutoff to the
      // freshest publish time we've now toasted.
      for (const a of fresh) seenIdsRef.current.add(a.id);
      const freshestMs = fresh.reduce(
        (acc, a) => (a.publishedAtMs > acc ? a.publishedAtMs : acc),
        sinceMsRef.current ?? 0,
      );
      sinceMsRef.current = freshestMs;

      // Newest first; cap to MAX_OPEN_TOASTS so a burst doesn't
      // queue up an unbounded list of records.
      const records: ToastRecord[] = fresh
        .slice()
        .sort((a, b) => b.publishedAtMs - a.publishedAtMs)
        .map((article) => ({ key: article.id, article }));

      setToasts((current) => {
        const newKeys = new Set(records.map((r) => r.key));
        // Drop any in-flight record with the same id so a re-
        // surfaced article (e.g. after a severity-floor toggle
        // resets the dedupe set) replaces the old toast in place
        // rather than rendering twice with the same React key.
        const carried = current.filter((r) => !newKeys.has(r.key));
        return [...records, ...carried].slice(0, MAX_OPEN_TOASTS);
      });

      // Fire the open callback once per surfaced record. Radix
      // Toast does NOT call its own `onOpenChange(true)` on the
      // initial render (Toasts mount in the open state), so this
      // hook is the only signal a consumer sees for "the banner
      // surfaced this article".
      for (const r of records) {
        onToastOpenRef.current?.(r.article);
      }

      // Mirror the freshest item into the global news store so
      // sibling panels can pick it up without re-fetching.
      const freshestArticle = records[0]?.article;
      if (freshestArticle) {
        setBreaking({
          id: freshestArticle.id,
          title: freshestArticle.title,
          source: freshestArticle.source,
          publishedAtMs: freshestArticle.publishedAtMs,
          ...(freshestArticle.url ? { url: freshestArticle.url } : {}),
          ...(freshestArticle.summary ? { summary: freshestArticle.summary } : {}),
          ...(freshestArticle.severity
            ? { severity: freshestArticle.severity }
            : {}),
        });
      }
    };

    // Kick the first poll immediately so the banner doesn't wait
    // a full interval to render.
    void poll();
    const handle = setInterval(() => {
      void poll();
    }, pollIntervalMs);

    return () => {
      cancelled = true;
      clearInterval(handle);
    };
  }, [
    hasTier,
    loader,
    pollIntervalMs,
    props.query,
    setBreaking,
  ]);

  const handleToastOpenChange = useCallback(
    (record: ToastRecord, open: boolean) => {
      // Radix Toast mounts in the open state, so the only
      // transition we observe here is open → closed (auto-
      // dismiss, swipe, or explicit close). The "opened" signal
      // is fired from the polling loop instead — see
      // `onToastOpenRef.current?.()` after a record is added.
      if (open) return;
      setToasts((current) => current.filter((r) => r.key !== record.key));
    },
    [],
  );

  const handleClickThrough = useCallback(
    (article: NewsArticle) => {
      props.onToastClick?.(article);
    },
    [props],
  );

  if (isHidden) return <></>;
  if (!hasTier(REQUIRED_TIER)) {
    return (
      <div
        data-panel-id={BREAKING_NEWS_PANEL_ID}
        data-state="locked"
        role="alert"
        className="text-xs text-[var(--pellucid-warn)]"
      >
        Locked — breaking-news banner requires tier {REQUIRED_TIER} or higher.
      </div>
    );
  }

  return (
    <ToastProvider
      swipeDirection="right"
      duration={autoDismissMs}
      label="Breaking news"
    >
      <div
        data-panel-id={BREAKING_NEWS_PANEL_ID}
        data-state={toasts.length > 0 ? "active" : lastError ? "error" : "idle"}
        data-error-code={lastError?.code ?? ""}
        aria-live="polite"
        className="sr-only"
      >
        {toasts.length === 0
          ? lastError
            ? `Breaking news unavailable: ${lastError.message}`
            : "No breaking news."
          : `${toasts.length} breaking news ${
              toasts.length === 1 ? "alert" : "alerts"
            }.`}
      </div>
      {toasts.map((record) => (
        <BreakingToast
          key={record.key}
          record={record}
          onOpenChange={(open) => handleToastOpenChange(record, open)}
          onClickThrough={handleClickThrough}
        />
      ))}
      <ToastViewport />
    </ToastProvider>
  );
}

interface BreakingToastProps {
  record: ToastRecord;
  onOpenChange: (open: boolean) => void;
  onClickThrough: (article: NewsArticle) => void;
}

function BreakingToast({
  record,
  onOpenChange,
  onClickThrough,
}: BreakingToastProps): ReactElement {
  const { article } = record;
  const tone: ToastTone = toneFor(article.severity);
  return (
    <Toast
      tone={tone}
      onOpenChange={onOpenChange}
      data-component="BreakingToast"
      data-news-id={article.id}
      data-severity={article.severity ?? "info"}
    >
      <div className="flex flex-col gap-1">
        <ToastTitle>
          {article.url ? (
            <a
              href={article.url}
              target="_blank"
              rel="noreferrer"
              data-field="title-link"
              className="font-semibold underline-offset-2 hover:underline"
              onClick={() => onClickThrough(article)}
            >
              {article.title}
            </a>
          ) : (
            <span data-field="title" className="font-semibold">
              {article.title}
            </span>
          )}
        </ToastTitle>
        {article.summary ? (
          <ToastDescription>{article.summary}</ToastDescription>
        ) : null}
        <div className="flex items-center justify-between gap-2 text-[10px] uppercase font-mono text-[var(--pellucid-fg-muted)]">
          <span data-field="source">{article.source}</span>
          {article.url ? (
            <ToastAction
              asChild
              altText={`Open: ${article.title}`}
            >
              <a
                href={article.url}
                target="_blank"
                rel="noreferrer"
                data-field="action-link"
                className="underline-offset-2 hover:underline"
                onClick={() => onClickThrough(article)}
              >
                Open
              </a>
            </ToastAction>
          ) : null}
        </div>
      </div>
    </Toast>
  );
}

/**
 * Map a news severity to the design-token Toast tone. Pure —
 * exported for the unit test that pins the mapping.
 */
export function toneFor(severity: SignalSeverity | undefined): ToastTone {
  switch (severity) {
    case "critical":
      return "danger";
    case "high":
      return "warning";
    case "warn":
      return "warning";
    case "info":
      return "info";
    default:
      return "info";
  }
}

// Register in the family registry on import.
registerPanel({
  id: BREAKING_NEWS_PANEL_ID,
  title: "Breaking news",
  blurb: "Auto-dismissing toast for high-severity breaking signals.",
  component: BreakingNewsBanner as React.ComponentType<unknown>,
  cacheKeys: ["news:articles:list:v1"],
  minTier: REQUIRED_TIER,
  variants: "*",
});
