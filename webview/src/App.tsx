import {
  type ReactElement,
  useEffect,
  useRef,
  useState,
} from "react";

import { runBoot, type BootHandle } from "./app/boot";
import { useBootStore, type BootPhase } from "./state/useBootStore";
import { useVariantStore } from "./state/useVariantStore";

export interface AppProps {
  /** Test hook: pass `false` to skip the boot machine. */
  autoBoot?: boolean;
  /** Optional URL search override consumed by P5. */
  urlSearch?: string;
}

const PHASE_LABELS: Record<BootPhase, string> = {
  idle: "Idle",
  "p1-storage-i18n-ml-init": "P1 — Storage + reactions",
  "p2-bootstrap-fast-slow": "P2 — Sidecar handshake",
  "p3-clerk-auth": "P3 — Clerk auth",
  "p4-panel-layout": "P4 — Panel layout",
  "p5-search-intel-url-state": "P5 — URL state",
  "p6-parallel-data-load": "P6 — Data load",
  "p7-smart-poll-loop": "P7 — Poll loop",
  "p8-desktop-updater": "P8 — Updater",
  ready: "Ready",
  errored: "Errored",
};

const PHASE_ORDER: BootPhase[] = [
  "idle",
  "p1-storage-i18n-ml-init",
  "p2-bootstrap-fast-slow",
  "p3-clerk-auth",
  "p4-panel-layout",
  "p5-search-intel-url-state",
  "p6-parallel-data-load",
  "p7-smart-poll-loop",
  "p8-desktop-updater",
  "ready",
];

/**
 * Root component of the Pellucid webview.
 *
 * At T1.11 the app spins up the 8-phase boot machine on mount. The DOM
 * surfaces a phase indicator (`[data-testid="boot-phase"]`) the e2e
 * suite observes as the boot progresses. Variant changes at the
 * Zustand layer are mirrored onto the `<html>` `data-variant`
 * attribute so the Tailwind theme tokens swap live.
 */
export function App({ autoBoot = true, urlSearch }: AppProps): ReactElement {
  const phase = useBootStore((s) => s.phase);
  const errorMessage = useBootStore((s) => s.errorMessage);
  const variant = useVariantStore((s) => s.variant);
  const handleRef = useRef<BootHandle | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);

  // Mirror variant onto the html attribute so the variant CSS swaps.
  useEffect(() => {
    if (typeof document === "undefined") return;
    document.documentElement.setAttribute("data-variant", variant);
  }, [variant]);

  // Drive the boot machine on mount.
  useEffect(() => {
    if (!autoBoot) return;
    let cancelled = false;
    (async () => {
      try {
        const opts = urlSearch !== undefined ? { urlSearch } : {};
        const handle = await runBoot(opts);
        if (cancelled) {
          await handle.shutdown();
          return;
        }
        handleRef.current = handle;
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setBootError(message);
      }
    })();
    return () => {
      cancelled = true;
      handleRef.current?.shutdown().catch((err) => {
        console.warn("[App] boot shutdown threw:", err);
      });
      handleRef.current = null;
    };
  }, [autoBoot, urlSearch]);

  const reachedIndex = PHASE_ORDER.indexOf(phase);

  return (
    <main data-testid="app-root" className="min-h-screen p-4">
      <header className="mb-4">
        <h1 data-testid="app-title" className="text-xl font-semibold">
          Pellucid
        </h1>
        <p
          data-testid="app-tagline"
          className="text-sm text-[var(--pellucid-fg-muted)]"
        >
          Real-time situational awareness console.
        </p>
      </header>
      <section
        data-testid="boot-progress"
        aria-label="Boot progress"
        className="mb-4 rounded-[var(--pellucid-radius-md)] border border-[var(--pellucid-border)] bg-[var(--pellucid-surface-raised)] p-3"
      >
        <div className="flex items-center justify-between text-xs uppercase tracking-wide text-[var(--pellucid-fg-muted)]">
          <span>Boot</span>
          <span data-testid="boot-phase" data-phase={phase}>
            {PHASE_LABELS[phase]}
          </span>
        </div>
        <ol
          data-testid="boot-trail"
          className="mt-2 flex flex-wrap gap-1 text-[10px] uppercase tracking-wide"
        >
          {PHASE_ORDER.map((p, i) => (
            <li
              key={p}
              data-phase={p}
              data-reached={i <= reachedIndex || phase === "ready"}
              className={
                i <= reachedIndex || phase === "ready"
                  ? "rounded bg-[var(--pellucid-accent)] px-1.5 py-0.5 text-[var(--pellucid-accent-fg)]"
                  : "rounded border border-[var(--pellucid-border)] px-1.5 py-0.5 text-[var(--pellucid-fg-muted)]"
              }
            >
              {p === "idle" ? "P0" : p === "ready" ? "✓" : p.slice(0, 2).toUpperCase()}
            </li>
          ))}
        </ol>
        {phase === "errored" || bootError !== null ? (
          <p
            data-testid="boot-error"
            className="mt-2 text-xs text-[var(--pellucid-danger)]"
          >
            Boot failed: {errorMessage ?? bootError}
          </p>
        ) : null}
      </section>
      <section
        data-testid="app-shell"
        aria-label="Application shell"
        className="rounded-[var(--pellucid-radius-md)] border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3"
      >
        <p className="text-sm">
          Variant:{" "}
          <span data-testid="active-variant" className="font-mono">
            {variant}
          </span>
        </p>
      </section>
    </main>
  );
}
