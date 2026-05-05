import {
  useEffect,
  useMemo,
  useState,
  type ReactElement,
} from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  loadRegional,
  type RegionRollup,
  type RegionalOutcome,
  type RegionalQuery,
} from "../../data/loaders/intel/regional";
import { IntelEntityChip, registerPanel } from "./index";

/** Stable id for `usePanelStore` registration. */
export const REGIONAL_BOARD_PANEL_ID = "intel/regional";

/** Tier gate. Both upstream cache reads are anonymous. */
export const REQUIRED_TIER = 0;

export interface RegionalIntelligenceBoardProps {
  /** Optional pre-filter passed to the loader. */
  query?: RegionalQuery;
  /** Override the loader (testing). */
  load?: typeof loadRegional;
}

type ViewState =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: RegionalOutcome };

/**
 * Regional intelligence board — M3 family 4.1, T4.1.6.
 *
 * Renders the per-region rollup the
 * `intelligence/v1/regional` handler composes from ACLED +
 * GDELT cache slots. The board is a 3-column grid of region
 * cards; each card surfaces its event + incident totals, top
 * actors as `IntelEntityChip`s, and the per-country breakdown
 * sorted by combined volume desc.
 */
export function RegionalIntelligenceBoard(
  props: RegionalIntelligenceBoardProps,
): ReactElement {
  const hasTier = useAuthStore((s) => s.hasTier);
  const setLayout = usePanelStore((s) => s.setLayout);
  const isHidden = usePanelStore((s) => s.isHidden(REGIONAL_BOARD_PANEL_ID));

  const [view, setView] = useState<ViewState>({ kind: "loading" });
  const [activeRegion, setActiveRegion] = useState<string | null>(
    props.query?.region ?? null,
  );

  const effectiveQuery = useMemo<RegionalQuery>(() => {
    return activeRegion ? { region: activeRegion } : {};
  }, [activeRegion]);

  useEffect(() => {
    setLayout(REGIONAL_BOARD_PANEL_ID, { rowSpan: 2, colSpan: 4 });
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
    const loader = props.load ?? loadRegional;
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
      data-panel-id={REGIONAL_BOARD_PANEL_ID}
      className="rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 flex flex-col gap-3"
      aria-label="Regional intelligence board"
    >
      <header className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">Regional intel</h3>
        {activeRegion ? (
          <button
            type="button"
            data-action="clear-region"
            onClick={() => setActiveRegion(null)}
            className="text-[11px] uppercase font-mono text-[var(--pellucid-muted)] hover:text-[var(--pellucid-fg)]"
          >
            Show all regions
          </button>
        ) : null}
      </header>
      {renderBody(view, activeRegion, setActiveRegion)}
    </section>
  );
}

function renderBody(
  view: ViewState,
  active: string | null,
  setActive: (next: string | null) => void,
): ReactElement {
  if (view.kind === "loading") {
    return (
      <div
        role="status"
        aria-live="polite"
        className="text-xs text-[var(--pellucid-muted)]"
      >
        Loading regional intel…
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
    if (out.response.regions.length === 0) {
      return (
        <div
          data-state="empty"
          role="status"
          className="text-xs text-[var(--pellucid-muted)]"
        >
          No regional signal at the moment.
        </div>
      );
    }
    return (
      <div data-state="ready" className="flex flex-col gap-2">
        <ul
          className="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3"
          aria-label={`${out.response.regions.length} regions`}
        >
          {out.response.regions.map((r) => (
            <li key={r.region}>
              <RegionCard
                region={r}
                active={active === r.region}
                onSelect={() => setActive(active === r.region ? null : r.region)}
              />
            </li>
          ))}
        </ul>
        <footer
          data-component="RegionalBoardFooter"
          className="flex items-center justify-between text-[11px] text-[var(--pellucid-muted)]"
        >
          <span>{out.response.regions.length} regions</span>
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
        <strong>Regional intel pipeline offline.</strong>
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

interface RegionCardProps {
  region: RegionRollup;
  active: boolean;
  onSelect: () => void;
}

function RegionCard({
  region,
  active,
  onSelect,
}: RegionCardProps): ReactElement {
  return (
    <article
      data-component="RegionCard"
      data-region={region.region}
      data-region-active={active ? "true" : "false"}
      className={`flex flex-col gap-2 rounded-md border p-2 ${
        active
          ? "border-[var(--pellucid-info)] bg-[var(--pellucid-surface-raised)]"
          : "border-[var(--pellucid-border)] bg-[var(--pellucid-surface)]"
      }`}
    >
      <header className="flex items-baseline justify-between gap-2">
        <button
          type="button"
          data-field="region-name"
          aria-pressed={active ? "true" : "false"}
          onClick={onSelect}
          className="text-sm font-semibold hover:underline"
        >
          {region.region}
        </button>
        <span
          data-field="totals"
          className="text-[11px] font-mono text-[var(--pellucid-muted)]"
        >
          {region.totalEvents}E · {region.totalIncidents}I
        </span>
      </header>
      {region.topActors.length > 0 ? (
        <div className="flex flex-wrap gap-1" data-field="top-actors">
          {region.topActors.map((a) => (
            <IntelEntityChip
              key={a}
              entity={{ id: `actor:${a}`, kind: "actor", name: a }}
            />
          ))}
        </div>
      ) : null}
      {region.countries.length > 0 ? (
        <ul
          data-field="countries"
          aria-label={`${region.countries.length} countries`}
          className="flex flex-col gap-0.5"
        >
          {region.countries.map((c) => (
            <li
              key={c.name}
              data-component="CountryRollup"
              data-country={c.name}
              className="flex items-center justify-between text-[11px] text-[var(--pellucid-fg)]"
            >
              <span>{c.name}</span>
              <span className="font-mono text-[var(--pellucid-muted)]">
                {c.events}E · {c.incidents}I
              </span>
            </li>
          ))}
        </ul>
      ) : null}
    </article>
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
 * in the GDELT + Telegram intel panels.
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
  id: REGIONAL_BOARD_PANEL_ID,
  title: "Regional intel",
  blurb: "Per-region rollup of conflict events + GDELT incidents.",
  component: RegionalIntelligenceBoard as React.ComponentType<unknown>,
  cacheKeys: ["conflict:events-24h:v1", "conflict:incident-feed:v1"],
  minTier: REQUIRED_TIER,
  variants: "*",
});
