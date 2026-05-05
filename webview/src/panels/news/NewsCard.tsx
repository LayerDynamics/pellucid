import { type ReactElement } from "react";

import { cn } from "../../components/primitives/cn";
import { SignalSeverityBadge } from "./SignalSeverityBadge";

/**
 * One news article rendered as a card. Shared sub-component for
 * `NewsPanel`, `LiveNewsPanel`, and the M3 family showcase page.
 *
 * Props mirror the `NewsItem` shape from `useNewsStore` so the
 * card can be fed directly from store state without re-shaping.
 * The card is purely presentational — the parent panel handles
 * filtering, sort order, and click-through routing.
 */

/** Minimum data the card needs to render. Matches the
 *  intersection of every news handler's row shape (the canonical
 *  superset is `NewsItem` in `useNewsStore`). */
export interface NewsCardItem {
  /** Stable identifier — used as the React key by the parent. */
  id: string;
  /** Headline. Required. */
  title: string;
  /** Source attribution (publisher, feed name, agency). */
  source: string;
  /** Wall-clock ms when the article was published upstream. The
   *  card formats this to a relative string against `now`. */
  publishedAtMs: number;
  /** Click-through URL. When omitted the card renders the title
   *  as plain text (no anchor). */
  url?: string;
  /** Short snippet — typically the first ~200 chars of the
   *  article body or an upstream-provided summary. */
  summary?: string;
  /** Severity tag — propagated to the embedded
   *  [`SignalSeverityBadge`]. When omitted no badge renders. */
  severity?: "info" | "warn" | "high" | "critical";
}

export interface NewsCardProps {
  /** The article. */
  item: NewsCardItem;
  /** Wall-clock ms used as "now" for the relative timestamp.
   *  Defaults to `Date.now()`; tests override for determinism. */
  now?: number;
  /** Extra Tailwind classes. */
  className?: string;
  /** Click handler. When set, fires alongside the anchor's default
   *  navigation (do not preventDefault unless you want to suppress
   *  it). */
  onSelect?: (item: NewsCardItem) => void;
}

export function NewsCard(props: NewsCardProps): ReactElement {
  const { item, now = Date.now(), className, onSelect } = props;
  const titleNode = item.url ? (
    <a
      href={item.url}
      className="font-semibold text-[var(--pellucid-fg)] underline-offset-2 hover:underline"
      target="_blank"
      rel="noreferrer"
      onClick={() => onSelect?.(item)}
    >
      {item.title}
    </a>
  ) : (
    <span className="font-semibold text-[var(--pellucid-fg)]">
      {item.title}
    </span>
  );

  return (
    <article
      data-component="NewsCard"
      data-news-id={item.id}
      className={cn(
        "rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 text-xs",
        "flex flex-col gap-1",
        className,
      )}
      aria-label={`News: ${item.title}`}
    >
      <header className="flex items-start justify-between gap-2">
        <h4 className="leading-tight">{titleNode}</h4>
        {item.severity ? (
          <SignalSeverityBadge severity={item.severity} compact />
        ) : null}
      </header>
      <div className="flex items-center gap-2 text-[var(--pellucid-muted)]">
        <span data-field="source">{item.source}</span>
        <span aria-hidden="true">·</span>
        <time
          data-field="published"
          dateTime={new Date(item.publishedAtMs).toISOString()}
        >
          {formatRelative(item.publishedAtMs, now)}
        </time>
      </div>
      {item.summary ? (
        <p
          data-field="summary"
          className="mt-1 line-clamp-3 leading-snug text-[var(--pellucid-fg)]"
        >
          {item.summary}
        </p>
      ) : null}
    </article>
  );
}

/**
 * Format a wall-clock ms timestamp as a human-friendly relative
 * string against `now`. Pure — no Intl locale lookup, no
 * timezone math; the article timestamp is already absolute UTC
 * and we surface "8m ago" / "2h ago" / "3d ago" / "Apr 12".
 *
 * Exported so the unit test pins the boundaries.
 */
export function formatRelative(timestampMs: number, nowMs: number): string {
  const deltaMs = nowMs - timestampMs;
  if (deltaMs < 0) return "just now"; // future timestamp — clamp.
  const sec = Math.floor(deltaMs / 1000);
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}d ago`;
  // Older than a week — switch to absolute "Mon DD" shorthand.
  const d = new Date(timestampMs);
  const months = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
  ];
  return `${months[d.getUTCMonth()]} ${d.getUTCDate()}`;
}
