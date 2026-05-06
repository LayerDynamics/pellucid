import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen, waitFor } from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  FEAR_GREED_PANEL_ID,
  FearGreedPanel,
  REQUIRED_TIER,
  formatAssembledAtUtc,
  labelClass,
  labelText,
} from "./FearGreedPanel";
import type {
  FearGreedOutcome,
  FearGreedResponse,
} from "../../data/loaders/market/fear-greed";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const ASSEMBLED_AT_MS = Date.UTC(2026, 4, 5, 11, 59, 0);

const SAMPLE: FearGreedResponse = {
  score: 62,
  label: "greed",
  components: [
    { name: "volatility", score: 75, label: "greed", rationale: "VIX at 14.2" },
    { name: "momentum", score: 60, label: "greed", rationale: "5/8 advancing" },
    { name: "strength", score: 55, label: "neutral", rationale: "Avg +0.18%" },
    { name: "volume", score: 58, label: "greed", rationale: "ETF activity 1.18×" },
  ],
  assembledAtMs: ASSEMBLED_AT_MS,
  stale: false,
};

function readyLoader(response: FearGreedResponse) {
  return async () => ({ kind: "ready", response }) as FearGreedOutcome;
}

describe("FearGreedPanel — view states", () => {
  test("loading → ready transitions and renders dial + components", async () => {
    render(<FearGreedPanel load={readyLoader(SAMPLE)} />);
    expect(screen.getByText("Loading sentiment…")).toBeDefined();
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    expect(
      document.querySelector('[data-component="FearGreedDial"]'),
    ).not.toBeNull();
    expect(
      document.querySelector('[data-component="FearGreedComponents"]'),
    ).not.toBeNull();
  });

  test("composite score + label are surfaced in the header", async () => {
    render(<FearGreedPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(
        document.querySelector('[data-field="composite-score"]'),
      ).not.toBeNull();
    });
    expect(
      document.querySelector('[data-field="composite-score"]')?.textContent,
    ).toBe("62");
    expect(
      document.querySelector('[data-field="composite-label"]')?.textContent,
    ).toBe("Greed");
  });

  test("dial bar marker positions at the score percentage", async () => {
    render(<FearGreedPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(
        document.querySelector('[data-field="dial-marker"]'),
      ).not.toBeNull();
    });
    const marker = document.querySelector(
      '[data-field="dial-marker"]',
    ) as HTMLElement;
    expect(marker.style.left).toBe("62%");
    const bar = document.querySelector('[data-component="FearGreedDialBar"]');
    expect(bar?.getAttribute("aria-valuenow")).toBe("62");
  });

  test("dial bar marker clamps to [0, 100]", async () => {
    render(
      <FearGreedPanel load={readyLoader({ ...SAMPLE, score: 150 })} />,
    );
    await waitFor(() => {
      expect(
        document.querySelector('[data-field="dial-marker"]'),
      ).not.toBeNull();
    });
    const marker = document.querySelector(
      '[data-field="dial-marker"]',
    ) as HTMLElement;
    expect(marker.style.left).toBe("100%");
  });

  test("renders one row per component with the rationale string", async () => {
    render(<FearGreedPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(
        document.querySelector('[data-component="FearGreedComponentRow"]'),
      ).not.toBeNull();
    });
    const rows = document.querySelectorAll(
      '[data-component="FearGreedComponentRow"]',
    );
    expect(rows.length).toBe(4);
    const vix = document.querySelector(
      '[data-component-name="volatility"]',
    );
    expect(
      vix?.querySelector('[data-field="component-rationale"]')?.textContent,
    ).toBe("VIX at 14.2");
  });

  test("503 outage path renders the outage banner with retry-after", async () => {
    render(
      <FearGreedPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as FearGreedOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
  });

  test("generic error renders message + error-code data attribute", async () => {
    render(
      <FearGreedPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db locked",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as FearGreedOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("stale snapshot footer surfaces the 'cached snapshot' hint", async () => {
    render(
      <FearGreedPanel load={readyLoader({ ...SAMPLE, stale: true })} />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });
});

describe("FearGreedPanel — registration + lifecycle", () => {
  test("registers itself with usePanelStore on mount", () => {
    render(<FearGreedPanel load={readyLoader(SAMPLE)} />);
    const layout = usePanelStore.getState().getLayout(FEAR_GREED_PANEL_ID);
    expect(layout?.id).toBe(FEAR_GREED_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(2);
  });

  test("hidden panels render nothing", () => {
    usePanelStore.getState().hide(FEAR_GREED_PANEL_ID);
    const { container } = render(
      <FearGreedPanel load={readyLoader(SAMPLE)} />,
    );
    expect(container.querySelector("[data-panel-id]")).toBeNull();
  });

  test("REQUIRED_TIER is 0 (anonymous)", () => {
    expect(REQUIRED_TIER).toBe(0);
  });
});

describe("labelText", () => {
  test("maps every label to the display string", () => {
    expect(labelText("extreme-fear")).toBe("Extreme fear");
    expect(labelText("fear")).toBe("Fear");
    expect(labelText("neutral")).toBe("Neutral");
    expect(labelText("greed")).toBe("Greed");
    expect(labelText("extreme-greed")).toBe("Extreme greed");
  });
});

describe("labelClass", () => {
  test("fear-side labels use the danger token", () => {
    expect(labelClass("fear")).toContain("danger");
    expect(labelClass("extreme-fear")).toContain("danger");
  });
  test("greed-side labels use the success token", () => {
    expect(labelClass("greed")).toContain("success");
    expect(labelClass("extreme-greed")).toContain("success");
  });
  test("neutral uses the muted token", () => {
    expect(labelClass("neutral")).toContain("muted");
  });
});

describe("formatAssembledAtUtc", () => {
  test("formats wall-clock ms as HH:MM UTC", () => {
    expect(formatAssembledAtUtc(Date.UTC(2026, 4, 5, 1, 2, 0))).toBe("01:02 UTC");
  });
  test("non-finite or zero → em-dash", () => {
    expect(formatAssembledAtUtc(0)).toBe("—");
    expect(formatAssembledAtUtc(Number.NaN)).toBe("—");
  });
});
