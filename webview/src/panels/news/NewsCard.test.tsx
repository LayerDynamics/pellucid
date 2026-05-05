import { afterEach, describe, expect, test } from "bun:test";
import {
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";

import { NewsCard, formatRelative, type NewsCardItem } from "./NewsCard";

afterEach(() => cleanup());

const NOW_MS = Date.UTC(2026, 4, 4, 12, 0, 0);
const HOUR = 60 * 60 * 1000;

const SAMPLE: NewsCardItem = {
  id: "n-1",
  title: "Major shipping route reopens",
  source: "Reuters",
  publishedAtMs: NOW_MS - 3 * HOUR,
  url: "https://example.com/article",
  summary: "First convoy passes after 14-day closure.",
  severity: "high",
};

describe("NewsCard", () => {
  test("renders title as anchor when url is set", () => {
    render(<NewsCard item={SAMPLE} now={NOW_MS} />);
    const link = screen.getByRole("link", { name: SAMPLE.title });
    expect(link).toBeDefined();
    expect(link.getAttribute("href")).toBe(SAMPLE.url ?? "");
    expect(link.getAttribute("rel") ?? "").toContain("noreferrer");
  });

  test("renders title as span when url is missing", () => {
    const { url: _url, ...itemNoUrl } = SAMPLE;
    render(<NewsCard item={itemNoUrl} now={NOW_MS} />);
    expect(screen.queryByRole("link")).toBeNull();
    expect(screen.getByText(SAMPLE.title)).toBeDefined();
  });

  test("renders source + relative time", () => {
    render(<NewsCard item={SAMPLE} now={NOW_MS} />);
    expect(screen.getByText("Reuters")).toBeDefined();
    expect(screen.getByText("3h ago")).toBeDefined();
  });

  test("renders summary when provided", () => {
    render(<NewsCard item={SAMPLE} now={NOW_MS} />);
    const summary = document.querySelector('[data-field="summary"]');
    expect(summary?.textContent).toBe(SAMPLE.summary ?? "");
  });

  test("omits summary when not provided", () => {
    const { summary: _s, ...without } = SAMPLE;
    render(<NewsCard item={without} now={NOW_MS} />);
    expect(document.querySelector('[data-field="summary"]')).toBeNull();
  });

  test("renders SignalSeverityBadge when severity is set", () => {
    render(<NewsCard item={SAMPLE} now={NOW_MS} />);
    expect(
      document.querySelector('[data-component="SignalSeverityBadge"]'),
    ).not.toBeNull();
  });

  test("omits severity badge when severity is not set", () => {
    const { severity: _sev, ...without } = SAMPLE;
    render(<NewsCard item={without} now={NOW_MS} />);
    expect(
      document.querySelector('[data-component="SignalSeverityBadge"]'),
    ).toBeNull();
  });

  test("onSelect fires on click", () => {
    const received: NewsCardItem[] = [];
    render(
      <NewsCard
        item={SAMPLE}
        now={NOW_MS}
        onSelect={(it) => {
          received.push(it);
        }}
      />,
    );
    fireEvent.click(screen.getByRole("link"));
    expect(received).toEqual([SAMPLE]);
  });

  test("data-news-id attribute is set for layout / e2e selectors", () => {
    render(<NewsCard item={SAMPLE} now={NOW_MS} />);
    const card = document.querySelector('[data-component="NewsCard"]');
    expect(card?.getAttribute("data-news-id")).toBe(SAMPLE.id);
  });
});

describe("formatRelative", () => {
  test("returns 'just now' for future timestamp (clamp)", () => {
    expect(formatRelative(NOW_MS + 5_000, NOW_MS)).toBe("just now");
  });

  test("seconds boundary < 60", () => {
    expect(formatRelative(NOW_MS - 30_000, NOW_MS)).toBe("30s ago");
    expect(formatRelative(NOW_MS - 59_000, NOW_MS)).toBe("59s ago");
  });

  test("minutes boundary 60s..3600s", () => {
    expect(formatRelative(NOW_MS - 60_000, NOW_MS)).toBe("1m ago");
    expect(formatRelative(NOW_MS - 59 * 60_000, NOW_MS)).toBe("59m ago");
  });

  test("hours boundary 1h..24h", () => {
    expect(formatRelative(NOW_MS - HOUR, NOW_MS)).toBe("1h ago");
    expect(formatRelative(NOW_MS - 23 * HOUR, NOW_MS)).toBe("23h ago");
  });

  test("days boundary 1d..6d", () => {
    expect(formatRelative(NOW_MS - 24 * HOUR, NOW_MS)).toBe("1d ago");
    expect(formatRelative(NOW_MS - 6 * 24 * HOUR, NOW_MS)).toBe("6d ago");
  });

  test("falls back to absolute month-day past one week", () => {
    // 10 days before 2026-05-04 UTC = 2026-04-24.
    expect(formatRelative(NOW_MS - 10 * 24 * HOUR, NOW_MS)).toBe("Apr 24");
  });
});
