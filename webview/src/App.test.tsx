import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen, waitFor } from "@testing-library/react";

import { App } from "./App";
import { __pellucidRuntimeInternals } from "./services/runtime";
import { useBootStore } from "./state/useBootStore";
import { usePanelStore } from "./state/usePanelStore";
import { useVariantStore } from "./state/useVariantStore";

beforeEach(() => {
  useBootStore.getState().reset();
  usePanelStore.getState().reset();
  useVariantStore.setState({ variant: "base", switching: false });
});

afterEach(() => {
  cleanup();
  __pellucidRuntimeInternals.reset();
});

describe("App (unit)", () => {
  test("renders the Pellucid heading", () => {
    render(<App autoBoot={false} />);
    expect(screen.getByTestId("app-title").textContent).toBe("Pellucid");
  });

  test("renders product tagline", () => {
    render(<App autoBoot={false} />);
    expect(
      screen.getByTestId("app-tagline").textContent ?? "",
    ).toContain("situational awareness");
  });

  test("mounts a single root <main>", () => {
    render(<App autoBoot={false} />);
    expect(screen.getAllByTestId("app-root")).toHaveLength(1);
  });

  test("phase indicator starts at Idle when autoBoot is disabled", () => {
    render(<App autoBoot={false} />);
    const phaseEl = screen.getByTestId("boot-phase");
    expect(phaseEl.getAttribute("data-phase")).toBe("idle");
    expect(phaseEl.textContent).toBe("Idle");
  });

  test("phase indicator surfaces the active variant", () => {
    useVariantStore.setState({ variant: "finance", switching: false });
    render(<App autoBoot={false} />);
    expect(screen.getByTestId("active-variant").textContent).toBe("finance");
  });

  test("autoBoot flips the phase indicator past idle", async () => {
    render(<App autoBoot urlSearch="" />);
    await waitFor(
      () => {
        const phase = screen
          .getByTestId("boot-phase")
          .getAttribute("data-phase");
        expect(phase).toBe("ready");
      },
      { timeout: 2_000 },
    );
  });

  test("urlSearch=?variant=tech is applied during boot", async () => {
    render(<App autoBoot urlSearch="?variant=tech" />);
    await waitFor(
      () => {
        expect(useVariantStore.getState().variant).toBe("tech");
      },
      { timeout: 2_000 },
    );
  });
});
