/**
 * Tests for the shared testing-library utilities in `test/utils.tsx`.
 * Every helper exported from `utils.tsx` exercised at least once so the
 * universal test mandate (unit coverage of every public function) is met.
 */

import { describe, expect, test } from "bun:test";
import { cleanup, screen } from "@testing-library/react";

import { AllProviders, flush, renderApp } from "./utils";

describe("test/utils.tsx (unit)", () => {
  test("renderApp wraps children in the provider stack", () => {
    const { getByTestId } = renderApp(
      <span data-testid="probe">hello</span>,
    );
    expect(getByTestId("probe").textContent).toBe("hello");
    cleanup();
  });

  test("renderApp returns a userEvent instance", () => {
    const { user } = renderApp(<span data-testid="probe">x</span>);
    expect(typeof user.click).toBe("function");
    expect(typeof user.type).toBe("function");
    cleanup();
  });

  test("AllProviders renders children", () => {
    renderApp(
      <AllProviders>
        <span data-testid="provided">ok</span>
      </AllProviders>,
    );
    expect(screen.getAllByTestId("provided").length).toBeGreaterThanOrEqual(1);
    cleanup();
  });

  test("flush resolves on the next microtask", async () => {
    let order = 0;
    queueMicrotask(() => {
      order = 1;
    });
    await flush();
    expect(order).toBe(1);
  });
});
