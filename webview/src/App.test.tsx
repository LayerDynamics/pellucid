import { describe, expect, test } from "bun:test";
import { render, screen, cleanup } from "@testing-library/react";

import { App } from "./App";

describe("App (unit)", () => {
  test("renders the Pellucid heading", () => {
    render(<App />);
    expect(screen.getByTestId("app-title").textContent).toBe("Pellucid");
    cleanup();
  });

  test("respects custom initialMessage prop", () => {
    render(<App initialMessage="Pellucid · Tech Variant" />);
    expect(screen.getByTestId("app-title").textContent).toBe("Pellucid · Tech Variant");
    cleanup();
  });

  test("renders product tagline", () => {
    render(<App />);
    expect(screen.getByTestId("app-tagline").textContent).toContain("situational awareness");
    cleanup();
  });

  test("mounts a single root <main>", () => {
    render(<App />);
    expect(screen.getAllByTestId("app-root")).toHaveLength(1);
    cleanup();
  });
});
