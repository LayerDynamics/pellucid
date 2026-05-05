import {
  useEffect,
  useMemo,
  useState,
  type ReactElement,
} from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadCountryDeepDive,
  type CountryDeepDiveOutcome,
  type CountryDeepDiveQuery,
  type CountryDeepDiveResponse,
} from "../../data/loaders/intel/country-deep-dive";
import { IntelEntityChip, NewsCard, registerPanel } from "./index";

/** Stable id for `usePanelStore` registration. */
export const COUNTRY_DEEP_DIVE_PANEL_ID = "intel/country-deep-dive";

/** Tier gate. The handler reads three FAST-tier slots — no
 *  upstream side-effects — so the panel ships at tier 0. */
export const REQUIRED_TIER = 0;

/** Default country shown on first mount when the parent layout
 *  doesn't supply one. Matches the ACLED basket so the demo
 *  variant always has data to render. */
export const DEFAULT_COUNTRY = "Iran";

export interface CountryDeepDivePanelProps {
  /** Initial / pre-set country. Defaults to [`DEFAULT_COUNTRY`]. */
  country?: string;
  /** Override the loader (testing). */
  load?: typeof loadCountryDeepDive;
  /** Wall-clock ms used as "now" by the relative-time formatter. */
  now?: number;
  /** Optional pre-set actor + feed caps forwarded to the loader. */
  actors?: number;
  /** Optional feed-cap forwarded to the loader. */
  limit?: number;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: CountryDeepDiveOutcome };

/**
 * Country deep-dive panel — M3 family 4.1, T4.1.7.
 *
 * Renders the three-section payload the
 * `intelligence/v1/country-deep-dive` handler composes for one
 * country: top actors (ACLED), recent articles (GDELT), and
 * matching Telegram chatter. The header has a country picker
 * (free-form text input + small "presets" basket) bound to the
 * loader's required `?country=` knob.
 */
export function CountryDeepDivePanel(
  props: CountryDeepDivePanelProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(COUNTRY_DEEP_DIVE_PANEL_ID));

  const [country, setCountry] = useState<string>(props.country ?? DEFAULT_COUNTRY);
  const [view, setView] = useState<ViewState>({ kind: "loading" });

  const effectiveQuery = useMemo<CountryDeepDiveQuery>(() => {
    const q: CountryDeepDiveQuery = { country };
    if (typeof props.actors === "number") q.actors = props.actors;
    if (typeof props.limit === "number") q.limit = props.limit;
    return q;
  }, [country, props.actors, props.limit]);

  useEffect(() => {
    setLayout(COUNTRY_DEEP_DIVE_PANEL_ID, { rowSpan: 3, colSpan: 4 });
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
    const loader = props.load ?? loadCountryDeepDive;
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
      data-panel-id={COUNTRY_DEEP_DIVE_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Country deep dive"
    >
      <header className="flex flex-col gap-1">
        <div className="flex items-baseline justify-between gap-2">
          <h3 className="text-sm font-semibold">Country deep dive</h3>
          <CountryPicker value={country} onChange={setCountry} />
        </div>
        <CountryPresets active={country} onSelect={setCountry} />
      </header>
      {renderBody(view, props.now ?? Date.now())}
    </section>
  );
}

const PRESET_COUNTRIES = ["Iran", "Israel", "Ukraine", "Russia", "China"];

function renderBody(view: ViewState, now: number): ReactElement {
  if (view.kind === "loading") {
    return (
      <div
        role="status"
        aria-live="polite"
        className="text-xs text-[var(--pellucid-muted)]"
      >
        Loading deep dive…
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
    if (
      r.actors.length === 0 &&
      r.articles.length === 0 &&
      r.telegram.length === 0
    ) {
      return (
        <div
          data-state="empty"
          role="status"
          className="text-xs text-[var(--pellucid-muted)]"
        >
          No deep-dive signal for {r.country} right now.
        </div>
      );
    }
    return (
      <div data-state="ready" className="flex flex-col gap-3">
        <Totals response={r} />
        <Section title="Top actors" countAttr="actors">
          {r.actors.length === 0 ? (
            <p className="text-[11px] text-[var(--pellucid-muted)]">
              No ACLED actors for this country.
            </p>
          ) : (
            <ul
              className="flex flex-col gap-1"
              aria-label={`${r.actors.length} actors`}
            >
              {r.actors.map((a) => (
                <li
                  key={a.name}
                  data-component="DeepDiveActor"
                  data-actor={a.name}
                  className="flex items-center justify-between text-[11px]"
                >
                  <IntelEntityChip
                    entity={{
                      id: `actor:${a.name}`,
                      kind: "actor",
                      name: a.name,
                    }}
                  />
                  <span className="font-mono text-[var(--pellucid-muted)]">
                    {a.events}E · {a.totalFatalities}F
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Section>
        <Section title="Articles" countAttr="articles">
          {r.articles.length === 0 ? (
            <p className="text-[11px] text-[var(--pellucid-muted)]">
              No GDELT articles tagged to this country.
            </p>
          ) : (
            <ul
              className="flex flex-col gap-2"
              aria-label={`${r.articles.length} articles`}
            >
              {r.articles.map((a) => (
                <li key={a.url} data-component="DeepDiveArticle">
                  <NewsCard
                    item={{
                      id: a.url,
                      title: a.title,
                      source: a.domain,
                      publishedAtMs: parseGdeltSeenDate(a.seenDate) ?? now,
                      url: a.url,
                    }}
                    now={now}
                  />
                </li>
              ))}
            </ul>
          )}
        </Section>
        <Section title="Telegram" countAttr="telegram">
          {r.telegram.length === 0 ? (
            <p className="text-[11px] text-[var(--pellucid-muted)]">
              No Telegram chatter mentioning this country.
            </p>
          ) : (
            <ul
              className="flex flex-col gap-2"
              aria-label={`${r.telegram.length} messages`}
            >
              {r.telegram.map((t) => (
                <li
                  key={t.dataPost}
                  data-component="DeepDiveTelegram"
                  data-data-post={t.dataPost}
                >
                  <NewsCard
                    item={{
                      id: t.dataPost,
                      title:
                        t.text.split("\n")[0]?.slice(0, 120) || t.dataPost,
                      source: `@${t.channel}`,
                      publishedAtMs: parseIsoOrNow(t.datetime, now),
                      url: t.url,
                      summary: t.text.slice(0, 280),
                    }}
                    now={now}
                  />
                </li>
              ))}
            </ul>
          )}
        </Section>
        <footer
          data-component="DeepDiveFooter"
          className="flex items-center justify-between text-[11px] text-[var(--pellucid-muted)]"
        >
          <span data-field="region">Region: {r.region}</span>
          <span data-field="assembled-at">
            Snapshot: {formatAssembledAt(r.assembledAtMs)}
          </span>
          {r.stale ? (
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
        <strong>Deep-dive pipeline offline.</strong>
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

function Totals({ response }: { response: CountryDeepDiveResponse }): ReactElement {
  return (
    <div
      data-component="DeepDiveTotals"
      className="flex items-center gap-3 text-[11px] font-mono text-[var(--pellucid-muted)]"
    >
      <span data-field="totals-events">{response.totals.events} events</span>
      <span aria-hidden="true">·</span>
      <span data-field="totals-incidents">
        {response.totals.incidents} incidents
      </span>
      <span aria-hidden="true">·</span>
      <span data-field="totals-messages">
        {response.totals.messages} messages
      </span>
    </div>
  );
}

function Section({
  title,
  countAttr,
  children,
}: {
  title: string;
  countAttr: string;
  children: ReactElement;
}): ReactElement {
  return (
    <section
      data-component="DeepDiveSection"
      data-section={countAttr}
      className="flex flex-col gap-1"
    >
      <h4 className="text-[11px] uppercase font-mono text-[var(--pellucid-muted)]">
        {title}
      </h4>
      {children}
    </section>
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
      className="flex items-center gap-1 text-[11px] uppercase font-mono text-[var(--pellucid-muted)]"
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

function CountryPresets({
  active,
  onSelect,
}: {
  active: string;
  onSelect: (next: string) => void;
}): ReactElement {
  return (
    <div
      role="toolbar"
      aria-label="Country presets"
      className="flex flex-wrap items-center gap-1"
    >
      {PRESET_COUNTRIES.map((c) => (
        <button
          key={c}
          type="button"
          data-country-preset={c}
          aria-pressed={active.toLowerCase() === c.toLowerCase() ? "true" : "false"}
          onClick={() => onSelect(c)}
          className="rounded-full border border-[var(--pellucid-border)] px-2 py-0.5 text-[11px] font-mono lowercase hover:opacity-80"
        >
          {c}
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

/** Parse a GDELT `YYYYMMDDTHHMMSSZ` timestamp into wall-clock ms.
 *  Returns `null` on malformed input. Pure, exported for unit tests. */
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

function parseIsoOrNow(datetime: string, now: number): number {
  const ms = Date.parse(datetime);
  return Number.isFinite(ms) ? ms : now;
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
  id: COUNTRY_DEEP_DIVE_PANEL_ID,
  title: "Country deep dive",
  blurb: "Per-country actors + articles + chatter, composed across feeds.",
  component: CountryDeepDivePanel as React.ComponentType<unknown>,
  cacheKeys: [
    "conflict:events-24h:v1",
    "conflict:incident-feed:v1",
    "telegram:recent-feed:v1",
  ],
  minTier: REQUIRED_TIER,
  variants: "*",
});
