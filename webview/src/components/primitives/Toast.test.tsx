import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";

import {
  Toast,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
} from "./Toast";

afterEach(() => {
  cleanup();
});

describe("<Toast>", () => {
  test("renders title, description, and tone marker", () => {
    render(
      <ToastProvider>
        <Toast tone="success" open>
          <ToastTitle>Saved</ToastTitle>
          <ToastDescription>All good</ToastDescription>
        </Toast>
        <ToastViewport />
      </ToastProvider>,
    );
    expect(screen.getByText("Saved")).toBeDefined();
    expect(screen.getByText("All good")).toBeDefined();
    const toast = document.querySelector('[data-pellucid="toast"]');
    expect(toast?.getAttribute("data-tone")).toBe("success");
  });

  test("danger tone applies danger border class", () => {
    render(
      <ToastProvider>
        <Toast tone="danger" open>
          <ToastTitle>Boom</ToastTitle>
        </Toast>
        <ToastViewport />
      </ToastProvider>,
    );
    const toast = document.querySelector('[data-pellucid="toast"]');
    expect(toast?.className ?? "").toContain("border-[var(--pellucid-danger)]");
  });

  test("default tone is info", () => {
    render(
      <ToastProvider>
        <Toast open>
          <ToastTitle>Note</ToastTitle>
        </Toast>
        <ToastViewport />
      </ToastProvider>,
    );
    const toast = document.querySelector('[data-pellucid="toast"]');
    expect(toast?.getAttribute("data-tone")).toBe("info");
  });
});
