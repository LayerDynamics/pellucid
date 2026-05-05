import { afterEach, describe, expect, test } from "bun:test";
import {
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";

import {
  IntelEntityChip,
  formatConfidence,
  type IntelEntity,
  type IntelEntityKind,
} from "./IntelEntityChip";

afterEach(() => cleanup());

const COUNTRY: IntelEntity = {
  id: "country:US",
  kind: "country",
  name: "United States",
};

const INCIDENT: IntelEntity = {
  id: "incident:42",
  kind: "incident",
  name: "Suez closure",
  confidence: 0.76,
  href: "https://example.com/incident/42",
};

describe("IntelEntityChip", () => {
  test("renders entity name + kind glyph", () => {
    render(<IntelEntityChip entity={COUNTRY} />);
    expect(screen.getByText("United States")).toBeDefined();
    const chip = document.querySelector('[data-component="IntelEntityChip"]');
    expect(chip?.getAttribute("data-entity-kind")).toBe("country");
    expect(chip?.getAttribute("data-entity-id")).toBe("country:US");
  });

  test("renders confidence when provided", () => {
    render(<IntelEntityChip entity={INCIDENT} />);
    const conf = document.querySelector('[data-field="confidence"]');
    expect(conf?.textContent).toBe("76%");
  });

  test("omits confidence when not provided", () => {
    render(<IntelEntityChip entity={COUNTRY} />);
    expect(document.querySelector('[data-field="confidence"]')).toBeNull();
  });

  test("renders as <button> when onSelect is provided", () => {
    const captured: IntelEntity[] = [];
    render(
      <IntelEntityChip
        entity={INCIDENT}
        onSelect={(e) => {
          captured.push(e);
        }}
      />,
    );
    const btn = screen.getByRole("button");
    expect(btn.tagName).toBe("BUTTON");
    fireEvent.click(btn);
    expect(captured).toEqual([INCIDENT]);
  });

  test("renders as <span> when no onSelect", () => {
    render(<IntelEntityChip entity={COUNTRY} />);
    expect(screen.queryByRole("button")).toBeNull();
    const chip = document.querySelector('[data-component="IntelEntityChip"]');
    expect(chip?.tagName).toBe("SPAN");
  });

  test("each kind has a distinct data-entity-kind attribute", () => {
    const kinds: IntelEntityKind[] = [
      "country",
      "region",
      "actor",
      "organisation",
      "person",
      "event",
      "topic",
      "weapon",
      "incident",
    ];
    for (const k of kinds) {
      const { unmount } = render(
        <IntelEntityChip entity={{ id: `t:${k}`, kind: k, name: k }} />,
      );
      const chip = document.querySelector(
        `[data-entity-kind="${k}"]`,
      );
      expect(chip).not.toBeNull();
      unmount();
    }
  });

  test("aria-label communicates kind + name", () => {
    render(<IntelEntityChip entity={COUNTRY} />);
    const chip = document.querySelector('[data-component="IntelEntityChip"]');
    expect(chip?.getAttribute("aria-label")).toBe("country: United States");
  });
});

describe("formatConfidence", () => {
  test("rounds to nearest percent", () => {
    expect(formatConfidence(0.764)).toBe("76%");
    expect(formatConfidence(0.766)).toBe("77%");
  });

  test("clamps below 0", () => {
    expect(formatConfidence(-0.5)).toBe("0%");
  });

  test("clamps above 1", () => {
    expect(formatConfidence(2)).toBe("100%");
  });

  test("non-finite yields '?%'", () => {
    expect(formatConfidence(Number.NaN)).toBe("?%");
    expect(formatConfidence(Number.POSITIVE_INFINITY)).toBe("?%");
  });

  test("0 and 1 boundaries", () => {
    expect(formatConfidence(0)).toBe("0%");
    expect(formatConfidence(1)).toBe("100%");
  });
});
