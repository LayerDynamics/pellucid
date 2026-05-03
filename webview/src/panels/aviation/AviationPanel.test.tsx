import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen, waitFor } from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  AviationPanel,
  AVIATION_PANEL_ID,
  REQUIRED_TIER,
} from "./AviationPanel";
import type {
  FlightStatusOutcome,
  FlightStatusQuery,
} from "../../data/loaders/aviation";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const QUERY: FlightStatusQuery = {
  flight: "AA100",
  date: "2026-04-25",
  origin: "JFK",
};

function signedInProUser() {
  useAuthStore.getState().signIn({
    userId: "u",
    email: "u@example.com",
    clerkSessionToken: "tok",
    entitlements: {
      tier: 1, // Free → satisfies REQUIRED_TIER
      maxDashboards: 1,
      apiAccess: false,
      apiRateLimit: 60,
      prioritySupport: false,
      exportFormats: ["json"],
      validUntilMs: Date.now() + 60_000,
    },
  });
}

function delayedLoader(ms: number, outcome: FlightStatusOutcome) {
  return () =>
    new Promise<FlightStatusOutcome>((resolve) =>
      setTimeout(() => resolve(outcome), ms),
    );
}

describe("<AviationPanel>", () => {
  test("locked state when caller tier is below required", async () => {
    // No signIn — anonymous user. hasTier(0) returns false in this
    // store because no entitlements are loaded.
    render(<AviationPanel query={QUERY} load={delayedLoader(0, { kind: "ready", status: {
      flight: "AA100", scheduled_departure: "x", scheduled_arrival: "y", status: "active", origin: "JFK", destination: "LAX",
    } })} />);
    const alert = await screen.findByRole("alert");
    expect(alert.getAttribute("data-state")).toBe("locked");
    expect(alert.textContent).toContain(`tier ${REQUIRED_TIER}`);
  });

  test("loading state visible before loader resolves", async () => {
    signedInProUser();
    render(
      <AviationPanel
        query={QUERY}
        load={delayedLoader(50, {
          kind: "ready",
          status: {
            flight: "AA100",
            scheduled_departure: "x",
            scheduled_arrival: "y",
            status: "active",
            origin: "JFK",
            destination: "LAX",
          },
        })}
      />,
    );
    const status = await screen.findByRole("status");
    expect(status.textContent?.toLowerCase()).toContain("loading");
  });

  test("ready state renders flight metadata when loader returns success", async () => {
    signedInProUser();
    render(
      <AviationPanel
        query={QUERY}
        load={async () => ({
          kind: "ready",
          status: {
            flight: "AA100",
            scheduled_departure: "2026-04-25T12:00:00Z",
            scheduled_arrival: "2026-04-25T15:00:00Z",
            status: "active",
            origin: "JFK",
            destination: "LAX",
            departure_gate: "A12",
          },
        })}
      />,
    );
    const dl = await waitFor(() => {
      const node = document.querySelector('[data-state="ready"]');
      if (!node) throw new Error("ready not yet");
      return node;
    });
    expect(dl.textContent).toContain("active");
    expect(dl.textContent).toContain("JFK");
    expect(dl.textContent).toContain("LAX");
    expect(dl.textContent).toContain("A12");
  });

  test("error state renders the gateway error code", async () => {
    signedInProUser();
    render(
      <AviationPanel
        query={QUERY}
        load={async () => ({
          kind: "error",
          code: "upstream_failure",
          message: "flight not found",
          httpStatus: 502,
          retryAfterSecs: null,
        })}
      />,
    );
    const alert = await waitFor(() => {
      const node = document.querySelector('[data-state="error"]');
      if (!node) throw new Error("error not yet");
      return node;
    });
    expect(alert.getAttribute("data-error-code")).toBe("upstream_failure");
    expect(alert.textContent).toContain("Upstream unreachable");
    expect(alert.textContent).toContain("flight not found");
  });

  test("503 outage outcome renders Retry-In countdown", async () => {
    signedInProUser();
    render(
      <AviationPanel
        query={QUERY}
        load={async () => ({
          kind: "error",
          code: "upstream_failure",
          message: "outage",
          httpStatus: 503,
          retryAfterSecs: 30,
        })}
      />,
    );
    const alert = await waitFor(() => {
      const node = document.querySelector('[data-state="error"]');
      if (!node) throw new Error("error not yet");
      return node;
    });
    expect(alert.textContent).toContain("Retry in 30s");
  });

  test("registers itself in usePanelStore on mount", async () => {
    signedInProUser();
    render(
      <AviationPanel
        query={QUERY}
        load={async () => ({
          kind: "ready",
          status: {
            flight: "AA100",
            scheduled_departure: "x",
            scheduled_arrival: "y",
            status: "active",
            origin: "JFK",
            destination: "LAX",
          },
        })}
      />,
    );
    await waitFor(() => {
      const layout = usePanelStore.getState().getLayout(AVIATION_PANEL_ID);
      if (!layout) throw new Error("not registered yet");
      expect(layout.colSpan).toBe(2);
    });
  });

  test("hidden state collapses to empty fragment", () => {
    signedInProUser();
    usePanelStore.getState().hide(AVIATION_PANEL_ID);
    const { container } = render(
      <AviationPanel
        query={QUERY}
        load={async () => ({
          kind: "ready",
          status: {
            flight: "AA100",
            scheduled_departure: "x",
            scheduled_arrival: "y",
            status: "active",
            origin: "JFK",
            destination: "LAX",
          },
        })}
      />,
    );
    expect(container.querySelector('[data-panel-id="aviation/flight-status"]')).toBeNull();
  });
});
