import { useEffect, useRef, useState, type ReactElement } from "react";

import { cn } from "../../components/primitives/cn";
import { SignalSeverityBadge, type SignalSeverity } from "./SignalSeverityBadge";

/**
 * Horizontal marquee that cycles through the most recent
 * breaking-news headlines. Shared by `NewsPanel`,
 * `BreakingNewsBanner`, and the family showcase page.
 *
 * The ticker is **paused on hover** and **paused when the tab
 * is hidden** (Page Visibility API) so a user reading a headline
 * doesn't have it slide away mid-sentence and so background
 * tabs don't burn rAF cycles.
 *
 * Implementation note: this is a CSS-keyframe-driven marquee
 * (translateX from 0 to -100%) with the actual scroll handled
 * via inline `animation` style so the duration scales with the
 * concatenated headline width. The component duplicates the
 * headline list inline so the loop appears seamless.
 */

export interface BreakingTickerHeadline {
  /** Stable id (used as React key + data-attribute). */
  id: string;
  /** The headline text. */
  text: string;
  /** Optional severity — when set, a [`SignalSeverityBadge`]
   *  renders inline before the headline. */
  severity?: SignalSeverity;
  /** Click-through URL. */
  url?: string;
}

export interface BreakingTickerProps {
  /** Headlines to cycle through. The component preserves order
   *  and treats it as a circular queue. */
  headlines: BreakingTickerHeadline[];
  /** Pixels per second the strip scrolls. Defaults to 60 — fast
   *  enough to feel live, slow enough to read mid-headline. */
  pixelsPerSecond?: number;
  /** Extra Tailwind classes. */
  className?: string;
  /** Render flag injected by tests so the inline-animation style
   *  resolves deterministically without a real DOM measurement.
   *  When omitted the component computes a duration from the
   *  rendered strip width via a `ref`. */
  forceDurationSecs?: number;
}

const EMPTY_DURATION_SECS = 0;

export function BreakingTicker(props: BreakingTickerProps): ReactElement {
  const {
    headlines,
    pixelsPerSecond = 60,
    className,
    forceDurationSecs,
  } = props;
  const stripRef = useRef<HTMLDivElement | null>(null);
  const [durationSecs, setDurationSecs] = useState<number>(
    forceDurationSecs ?? EMPTY_DURATION_SECS,
  );
  const [paused, setPaused] = useState<boolean>(false);

  // Re-measure when the headline set changes; tests bypass the
  // measurement by passing `forceDurationSecs`.
  useEffect(() => {
    if (typeof forceDurationSecs === "number") {
      setDurationSecs(forceDurationSecs);
      return;
    }
    if (!stripRef.current || headlines.length === 0) {
      setDurationSecs(EMPTY_DURATION_SECS);
      return;
    }
    const width = stripRef.current.scrollWidth;
    // The strip is duplicated inline, so the scroll distance is
    // half the rendered width (one full cycle returns to the
    // original starting frame).
    const cycleDistance = width / 2;
    setDurationSecs(Math.max(1, cycleDistance / Math.max(1, pixelsPerSecond)));
  }, [headlines, pixelsPerSecond, forceDurationSecs]);

  // Pause on Page Visibility hidden so background tabs don't burn
  // CPU; resume on visible.
  useEffect(() => {
    const handleVisibility = () => {
      setPaused(document.visibilityState === "hidden");
    };
    handleVisibility(); // initial sync
    document.addEventListener("visibilitychange", handleVisibility);
    return () =>
      document.removeEventListener("visibilitychange", handleVisibility);
  }, []);

  if (headlines.length === 0) {
    return (
      <div
        data-component="BreakingTicker"
        data-state="empty"
        role="status"
        aria-live="polite"
        className={cn(
          "h-7 overflow-hidden rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] px-3 text-xs leading-7 text-[var(--pellucid-muted)]",
          className,
        )}
      >
        No breaking headlines.
      </div>
    );
  }

  // Inline style — keyframes are declared in the component-level
  // <style> block below so we don't depend on a global CSS file.
  const animationStyle =
    durationSecs > 0
      ? {
          animation: `pellucid-marquee ${durationSecs}s linear infinite`,
          animationPlayState: paused ? "paused" : "running",
        }
      : undefined;

  return (
    <div
      data-component="BreakingTicker"
      data-state="active"
      data-paused={paused ? "true" : "false"}
      className={cn(
        "h-7 overflow-hidden rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)]",
        className,
      )}
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() =>
        setPaused(document.visibilityState === "hidden")
      }
      role="region"
      aria-label="Breaking news ticker"
    >
      <style>{KEYFRAMES_CSS}</style>
      <div
        ref={stripRef}
        data-field="strip"
        className="flex h-full whitespace-nowrap will-change-transform"
        style={animationStyle}
      >
        {/* The strip is rendered twice so the marquee loops
            seamlessly. The second copy is `aria-hidden`. */}
        <TickerSegment headlines={headlines} />
        <TickerSegment headlines={headlines} ariaHidden />
      </div>
    </div>
  );
}

const KEYFRAMES_CSS = `
@keyframes pellucid-marquee {
  from { transform: translateX(0); }
  to   { transform: translateX(-50%); }
}
`;

function TickerSegment({
  headlines,
  ariaHidden = false,
}: {
  headlines: BreakingTickerHeadline[];
  ariaHidden?: boolean;
}): ReactElement {
  return (
    <div
      className="flex shrink-0 items-center gap-6 px-3 text-xs leading-7"
      aria-hidden={ariaHidden ? "true" : undefined}
    >
      {headlines.map((h) => (
        <span
          key={h.id}
          data-headline-id={h.id}
          className="flex items-center gap-2"
        >
          {h.severity ? (
            <SignalSeverityBadge severity={h.severity} compact />
          ) : null}
          {h.url ? (
            <a
              href={h.url}
              className="text-[var(--pellucid-fg)] hover:underline"
              target="_blank"
              rel="noreferrer"
            >
              {h.text}
            </a>
          ) : (
            <span className="text-[var(--pellucid-fg)]">{h.text}</span>
          )}
        </span>
      ))}
    </div>
  );
}
