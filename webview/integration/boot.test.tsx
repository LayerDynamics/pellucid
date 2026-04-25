import { describe, expect, test } from "bun:test";
import { render, screen, cleanup } from "@testing-library/react";
import { StrictMode } from "react";

import { App } from "../src/App";

describe("Boot (integration)", () => {
  test("App mounts inside StrictMode and reaches a stable DOM", () => {
    render(
      <StrictMode>
        <App />
      </StrictMode>,
    );

    const main = screen.getByTestId("app-root");
    expect(main.tagName).toBe("MAIN");
    expect(main.textContent).toContain("Pellucid");
    cleanup();
  });

  test("StrictMode double-invocation does not duplicate DOM nodes", () => {
    render(
      <StrictMode>
        <App />
      </StrictMode>,
    );

    expect(screen.getAllByTestId("app-title")).toHaveLength(1);
    expect(screen.getAllByTestId("app-tagline")).toHaveLength(1);
    cleanup();
  });
});
