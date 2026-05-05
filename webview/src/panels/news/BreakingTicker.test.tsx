import { afterEach, describe, expect, test } from "bun:test";
import {
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";

import {
  BreakingTicker,
  type BreakingTickerHeadline,
} from "./BreakingTicker";

afterEach(() => cleanup());

const HEADLINES: BreakingTickerHeadline[] = [
  { id: "h-1", text: "Suez closure update" },
  { id: "h-2", text: "BoJ holds rates", url: "https://example.com" },
  { id: "h-3", text: "Wildfire alert NorCal", severity: "high" },
];

describe("BreakingTicker", () => {
  test("renders empty state when headlines is empty", () => {
    render(<BreakingTicker headlines={[]} />);
    const el = document.querySelector('[data-component="BreakingTicker"]');
    expect(el?.getAttribute("data-state")).toBe("empty");
    expect(screen.getByText("No breaking headlines.")).toBeDefined();
  });

  test("renders strip + duplicates segment for seamless loop", () => {
    render(
      <BreakingTicker headlines={HEADLINES} forceDurationSecs={20} />,
    );
    const el = document.querySelector('[data-component="BreakingTicker"]');
    expect(el?.getAttribute("data-state")).toBe("active");
    // Each headline appears twice — once in the visible segment,
    // once in the aria-hidden duplicate that completes the loop.
    const occurrences = document.querySelectorAll(
      '[data-headline-id="h-1"]',
    );
    expect(occurrences.length).toBe(2);
  });

  test("renders headline as <a> when url is provided", () => {
    render(
      <BreakingTicker headlines={HEADLINES} forceDurationSecs={20} />,
    );
    const links = screen.getAllByRole("link", { name: "BoJ holds rates" });
    expect(links.length).toBeGreaterThan(0);
    const first = links[0];
    if (!first) throw new Error("expected at least one link");
    expect(first.getAttribute("href")).toBe("https://example.com");
  });

  test("renders SignalSeverityBadge when headline has severity", () => {
    render(
      <BreakingTicker headlines={HEADLINES} forceDurationSecs={20} />,
    );
    const badges = document.querySelectorAll(
      '[data-component="SignalSeverityBadge"]',
    );
    // Two copies of the strip → 2 badges for the one severity-tagged headline.
    expect(badges.length).toBe(2);
  });

  test("paused state toggles on hover", () => {
    render(
      <BreakingTicker headlines={HEADLINES} forceDurationSecs={20} />,
    );
    const el = document.querySelector(
      '[data-component="BreakingTicker"]',
    ) as HTMLElement;
    expect(el.getAttribute("data-paused")).toBe("false");
    fireEvent.mouseEnter(el);
    expect(el.getAttribute("data-paused")).toBe("true");
    fireEvent.mouseLeave(el);
    expect(el.getAttribute("data-paused")).toBe("false");
  });

  test("forceDurationSecs is honoured in inline animation style", () => {
    render(
      <BreakingTicker headlines={HEADLINES} forceDurationSecs={42} />,
    );
    const strip = document.querySelector(
      '[data-field="strip"]',
    ) as HTMLElement;
    expect(strip.style.animation).toContain("42s");
  });

  test("aria attributes mark a region with a name", () => {
    render(
      <BreakingTicker headlines={HEADLINES} forceDurationSecs={20} />,
    );
    const region = screen.getByRole("region", {
      name: "Breaking news ticker",
    });
    expect(region).toBeDefined();
  });

  test("custom className is merged onto the container", () => {
    render(
      <BreakingTicker
        headlines={HEADLINES}
        forceDurationSecs={20}
        className="my-test-class"
      />,
    );
    const el = document.querySelector(
      '[data-component="BreakingTicker"]',
    ) as HTMLElement;
    expect(el.className).toContain("my-test-class");
  });
});
