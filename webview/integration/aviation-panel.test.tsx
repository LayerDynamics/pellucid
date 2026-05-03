/**
 * Integration test for `<AviationPanel>` (T2.11).
 *
 * Mounts the panel against:
 * - Real `useAuthStore` populated with a Pro-tier entitlement.
 * - Real `usePanelStore` (verifies registration on mount).
 * - A spy loader that captures the params it was called with.
 *
 * Goal: prove the wiring from the panel's `query` prop down to the
 * loader is byte-faithful (the M1 demo collapses if the cache key
 * components diverge between the panel and the handler).
 */

import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { useAuthStore } from "../src/state/useAuthStore";
import { usePanelStore } from "../src/state/usePanelStore";
import {
  AviationPanel,
  AVIATION_PANEL_ID,
} from "../src/panels/aviation/AviationPanel";
import type {
  FlightStatusOutcome,
  FlightStatusQuery,
  loadFlightStatus as LoadFlightStatusFn,
} from "../src/data/loaders/aviation";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

function proUser() {
  useAuthStore.getState().signIn({
    userId: "user_test",
    email: "test@example.com",
    clerkSessionToken: "tok-test",
    entitlements: {
      tier: 1,
      maxDashboards: 5,
      apiAccess: true,
      apiRateLimit: 60,
      prioritySupport: false,
      exportFormats: ["json"],
      validUntilMs: Date.now() + 60_000,
    },
  });
}

function spyLoader(): {
  load: typeof LoadFlightStatusFn;
  calls: Array<{ query: FlightStatusQuery }>;
} {
  const calls: Array<{ query: FlightStatusQuery }> = [];
  const load: typeof LoadFlightStatusFn = async (query) => {
    calls.push({ query: { ...query } });
    const out: FlightStatusOutcome = {
      kind: "ready",
      status: {
        flight: query.flight,
        scheduled_departure: "2026-04-25T12:00:00Z",
        scheduled_arrival: "2026-04-25T15:00:00Z",
        status: "active",
        origin: query.origin,
        destination: "LAX",
      },
    };
    return out;
  };
  return { load, calls };
}

describe("<AviationPanel> integration", () => {
  test("loader is invoked with the exact query passed in props", async () => {
    proUser();
    const { load, calls } = spyLoader();
    const query: FlightStatusQuery = {
      flight: "AA100",
      date: "2026-04-25",
      origin: "JFK",
    };
    render(<AviationPanel query={query} load={load} />);
    await waitFor(() => {
      if (calls.length === 0) throw new Error("loader not called yet");
    });
    expect(calls.length).toBe(1);
    expect(calls[0]!.query).toEqual(query);
  });

  test("panel registers in usePanelStore on mount", async () => {
    proUser();
    const { load } = spyLoader();
    render(
      <AviationPanel
        query={{ flight: "AA100", date: "2026-04-25", origin: "JFK" }}
        load={load}
      />,
    );
    await waitFor(() => {
      const layout = usePanelStore.getState().getLayout(AVIATION_PANEL_ID);
      if (!layout) throw new Error("not yet");
      expect(layout.id).toBe(AVIATION_PANEL_ID);
      expect(layout.colSpan).toBe(2);
      expect(layout.hidden).toBe(false);
    });
  });

  test("loader re-runs when the query prop changes", async () => {
    proUser();
    const { load, calls } = spyLoader();
    const { rerender } = render(
      <AviationPanel
        query={{ flight: "AA100", date: "2026-04-25", origin: "JFK" }}
        load={load}
      />,
    );
    await waitFor(() => {
      if (calls.length < 1) throw new Error("first call missing");
    });
    rerender(
      <AviationPanel
        query={{ flight: "AA200", date: "2026-04-25", origin: "JFK" }}
        load={load}
      />,
    );
    await waitFor(() => {
      if (calls.length < 2) throw new Error("second call missing");
    });
    expect(calls[0]!.query.flight).toBe("AA100");
    expect(calls[1]!.query.flight).toBe("AA200");
  });

  test("anonymous user (entitlements missing) skips loader entirely", async () => {
    // No proUser() call — useAuthStore.entitlements is null.
    const { load, calls } = spyLoader();
    render(
      <AviationPanel
        query={{ flight: "AA100", date: "2026-04-25", origin: "JFK" }}
        load={load}
      />,
    );
    // Settle one tick — locked branch runs synchronously, loader
    // is NOT scheduled. Wait briefly to confirm no async call
    // sneaks in.
    await new Promise<void>((resolve) => setTimeout(resolve, 50));
    expect(calls.length).toBe(0);
  });
});
