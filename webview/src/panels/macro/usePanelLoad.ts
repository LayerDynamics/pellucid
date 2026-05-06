/**
 * Shared loader hook used by every macro/economy panel.
 * Encapsulates the locked / loading / ready view-state machine
 * + the cancellation-on-unmount pattern. The discriminated
 * `Outcome` shape is identical across every loader so the hook
 * is generic over its response payload.
 */

import { useEffect, useState } from "react";

import { useAuthStore } from "../../state/useAuthStore";

export type PanelOutcome<R> =
  | { kind: "ready"; response: R }
  | {
      kind: "error";
      code: string;
      message: string;
      httpStatus: number;
      retryAfterSecs: number | null;
    };

export type PanelView<R> =
  | { kind: "loading" }
  | { kind: "locked"; minTier: number }
  | { kind: "ready"; outcome: PanelOutcome<R> };

export interface UsePanelLoadOptions<R> {
  load: () => Promise<PanelOutcome<R>>;
  requiredTier: number;
  /** Re-run the loader whenever any of these dependencies change. */
  deps: ReadonlyArray<unknown>;
}

export function usePanelLoad<R>(opts: UsePanelLoadOptions<R>): PanelView<R> {
  const hasTier = useAuthStore((s) => s.hasTier);
  const [view, setView] = useState<PanelView<R>>({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    if (!hasTier(opts.requiredTier)) {
      setView({ kind: "locked", minTier: opts.requiredTier });
      return () => {
        cancelled = true;
      };
    }
    setView({ kind: "loading" });
    void opts.load().then((outcome) => {
      if (cancelled) return;
      setView({ kind: "ready", outcome });
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasTier, ...opts.deps]);

  return view;
}
