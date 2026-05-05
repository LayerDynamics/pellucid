import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";

import {
  SignalSeverityBadge,
  compareSeverity,
  type SignalSeverity,
} from "./SignalSeverityBadge";

afterEach(() => cleanup());

describe("SignalSeverityBadge", () => {
  test("renders default label per severity", () => {
    const { rerender } = render(<SignalSeverityBadge severity="info" />);
    expect(screen.getByText("Info")).toBeDefined();
    rerender(<SignalSeverityBadge severity="warn" />);
    expect(screen.getByText("Warn")).toBeDefined();
    rerender(<SignalSeverityBadge severity="high" />);
    expect(screen.getByText("High")).toBeDefined();
    rerender(<SignalSeverityBadge severity="critical" />);
    expect(screen.getByText("Critical")).toBeDefined();
  });

  test("compact mode swaps label for glyph", () => {
    render(<SignalSeverityBadge severity="critical" compact />);
    expect(screen.getByText("X")).toBeDefined();
    expect(screen.queryByText("Critical")).toBeNull();
  });

  test("explicit label overrides default", () => {
    render(<SignalSeverityBadge severity="warn" label="Watch" />);
    expect(screen.getByText("Watch")).toBeDefined();
    expect(screen.queryByText("Warn")).toBeNull();
  });

  test("data-severity + role/aria preserved across all four levels", () => {
    const severities: SignalSeverity[] = ["info", "warn", "high", "critical"];
    for (const sev of severities) {
      const { unmount } = render(<SignalSeverityBadge severity={sev} />);
      const el = document.querySelector(`[data-severity="${sev}"]`);
      expect(el).not.toBeNull();
      expect(el?.getAttribute("role")).toBe("img");
      expect(el?.getAttribute("aria-label")?.toLowerCase()).toContain(sev);
      unmount();
    }
  });

  test("custom className is merged onto the badge", () => {
    render(<SignalSeverityBadge severity="info" className="my-test-class" />);
    const el = document.querySelector('[data-component="SignalSeverityBadge"]');
    expect(el?.className).toContain("my-test-class");
  });
});

describe("compareSeverity", () => {
  test("orders descending by urgency", () => {
    const sorted: SignalSeverity[] = (
      ["info", "warn", "critical", "high"] as SignalSeverity[]
    ).sort(compareSeverity);
    expect(sorted).toEqual(["critical", "high", "warn", "info"]);
  });

  test("equal severities preserve relative order semantics (stable result)", () => {
    expect(compareSeverity("info", "info")).toBe(0);
    expect(compareSeverity("critical", "critical")).toBe(0);
  });

  test("sign matches direction", () => {
    expect(compareSeverity("critical", "info")).toBeLessThan(0);
    expect(compareSeverity("info", "critical")).toBeGreaterThan(0);
  });
});
