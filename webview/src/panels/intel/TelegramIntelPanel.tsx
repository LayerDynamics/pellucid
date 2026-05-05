import {
  useEffect,
  useMemo,
  useState,
  type ReactElement,
} from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadTelegramFeed,
  parseIsoDatetime,
  type FeedOutcome,
  type FeedQuery,
  type TelegramMessage,
} from "../../data/loaders/intel/telegram";
import { IntelEntityChip, NewsCard, registerPanel } from "./index";

/** Stable id for `usePanelStore` registration. */
export const TELEGRAM_INTEL_PANEL_ID = "intel/telegram";

/** Tier gate. The handler is anonymous — same shape as the
 *  GDELT panel — so the minimal Telegram public-channel feed
 *  ships at tier 0. */
export const REQUIRED_TIER = 0;

/** Default page size — matches the handler's `DEFAULT_LIMIT`. */
export const DEFAULT_LIMIT = 50;

export interface TelegramIntelPanelProps {
  /** Optional pre-filter passed to the loader. */
  query?: FeedQuery;
  /** Override the loader (testing). */
  load?: typeof loadTelegramFeed;
  /** Wall-clock ms used as "now" by the relative-time formatter. */
  now?: number;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: FeedOutcome };

/**
 * Telegram intelligence panel — M3 family 4.1, T4.1.5.
 *
 * Renders the FAST-tier snapshot
 * `seed_telegram_intel_min` writes from a small basket of
 * public Telegram channels. Each row renders as a [`NewsCard`]
 * (channel as source, datetime as the published time) plus a
 * channel chip + a views counter chip. A header row exposes
 * the channel basket as clickable filter chips bound to the
 * loader's `?channel=` query knob.
 */
export function TelegramIntelPanel(
  props: TelegramIntelPanelProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(TELEGRAM_INTEL_PANEL_ID));

  const [view, setView] = useState<ViewState>({ kind: "loading" });
  const [activeChannel, setActiveChannel] = useState<string | null>(
    props.query?.channel ?? null,
  );

  const effectiveQuery = useMemo<FeedQuery>(() => {
    const q: FeedQuery = { limit: props.query?.limit ?? DEFAULT_LIMIT };
    if (activeChannel) q.channel = activeChannel;
    return q;
  }, [props.query?.limit, activeChannel]);

  useEffect(() => {
    setLayout(TELEGRAM_INTEL_PANEL_ID, { rowSpan: 2, colSpan: 2 });
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
    const loader = props.load ?? loadTelegramFeed;
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
      data-panel-id={TELEGRAM_INTEL_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Telegram intelligence feed"
    >
      <header className="flex flex-col gap-1">
        <div className="flex items-baseline justify-between gap-2">
          <h3 className="text-sm font-semibold">Telegram intel</h3>
          {activeChannel ? (
            <button
              type="button"
              data-action="clear-channel"
              onClick={() => setActiveChannel(null)}
              className="text-[11px] uppercase font-mono text-[var(--pellucid-muted)] hover:text-[var(--pellucid-fg)]"
            >
              Clear filter
            </button>
          ) : null}
        </div>
        {channelsFromView(view).length > 0 ? (
          <ChannelBasket
            channels={channelsFromView(view)}
            active={activeChannel}
            onSelect={setActiveChannel}
          />
        ) : null}
      </header>
      {renderBody(view, props.now ?? Date.now())}
    </section>
  );
}

function channelsFromView(view: ViewState): string[] {
  if (view.kind !== "ready") return [];
  if (view.outcome.kind !== "ready") return [];
  return view.outcome.response.channels;
}

function renderBody(view: ViewState, now: number): ReactElement {
  if (view.kind === "loading") {
    return (
      <div
        role="status"
        aria-live="polite"
        className="text-xs text-[var(--pellucid-muted)]"
      >
        Loading Telegram intel…
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
          No messages match the current filters.
        </div>
      );
    }
    return (
      <div data-state="ready" className="flex flex-col gap-2">
        <ul
          className="flex flex-col gap-2"
          aria-label={`${out.response.rows.length} Telegram messages`}
        >
          {out.response.rows.map((row) => (
            <li key={row.dataPost}>
              <TelegramRow message={row} now={now} />
            </li>
          ))}
        </ul>
        <footer
          data-component="TelegramIntelFooter"
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
        <strong>Telegram pipeline offline.</strong>
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

interface TelegramRowProps {
  message: TelegramMessage;
  now: number;
}

function TelegramRow({ message, now }: TelegramRowProps): ReactElement {
  const publishedAtMs = parseIsoDatetime(message.datetime) ?? now;
  // Telegram messages are short — surface up to ~280 chars in
  // the summary and let NewsCard's line-clamp handle the rest.
  const summary =
    message.text.length > 280 ? `${message.text.slice(0, 280)}…` : message.text;
  return (
    <div
      data-component="TelegramRow"
      data-channel={message.channel}
      data-data-post={message.dataPost}
      className="flex flex-col gap-1"
    >
      <NewsCard
        item={{
          id: message.dataPost,
          title: message.text.split("\n")[0]?.slice(0, 120) || message.dataPost,
          source: `@${message.channel}`,
          publishedAtMs,
          url: message.url,
          summary,
        }}
        now={now}
      />
      <div className="flex flex-wrap items-center gap-1" data-field="entity-chips">
        <IntelEntityChip
          entity={{
            id: `topic:${message.channel}`,
            kind: "topic",
            name: message.channel,
          }}
        />
        {message.views ? (
          <span
            data-field="views"
            className="inline-flex items-center rounded-full border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] px-2 py-0.5 text-[10px] font-mono uppercase text-[var(--pellucid-muted)]"
          >
            {message.views} views
          </span>
        ) : null}
      </div>
    </div>
  );
}

function ChannelBasket({
  channels,
  active,
  onSelect,
}: {
  channels: string[];
  active: string | null;
  onSelect: (next: string | null) => void;
}): ReactElement {
  return (
    <div
      role="toolbar"
      aria-label="Channel basket"
      className="flex flex-wrap items-center gap-1"
    >
      {channels.map((c) => (
        <button
          key={c}
          type="button"
          data-channel-chip={c}
          aria-pressed={active === c ? "true" : "false"}
          onClick={() => onSelect(active === c ? null : c)}
          className="rounded-full border border-[var(--pellucid-border)] px-2 py-0.5 text-[11px] font-mono lowercase hover:opacity-80"
        >
          @{c}
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

/**
 * Format wall-clock ms as `HH:MM:SS UTC`. Pure — exported for
 * the unit test that pins the format. Mirrors the same helper
 * in `GdeltIntelPanel` so the two intel panels share the
 * footer chrome convention.
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
  id: TELEGRAM_INTEL_PANEL_ID,
  title: "Telegram intel",
  blurb: "Public-channel Telegram message stream.",
  component: TelegramIntelPanel as React.ComponentType<unknown>,
  cacheKeys: ["telegram:recent-feed:v1"],
  minTier: REQUIRED_TIER,
  variants: "*",
});
