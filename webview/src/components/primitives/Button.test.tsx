import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";

import { Button } from "./Button";

afterEach(() => {
  cleanup();
});

describe("<Button>", () => {
  test("renders solid + md by default", () => {
    render(<Button>Go</Button>);
    const btn = screen.getByRole("button", { name: "Go" });
    expect(btn.getAttribute("data-variant-button")).toBe("solid");
    expect(btn.getAttribute("data-size")).toBe("md");
    expect(btn.getAttribute("type")).toBe("button");
  });

  test("variant + size attributes flow through", () => {
    render(
      <Button variant="danger" size="lg">
        Danger
      </Button>,
    );
    const btn = screen.getByRole("button", { name: "Danger" });
    expect(btn.getAttribute("data-variant-button")).toBe("danger");
    expect(btn.getAttribute("data-size")).toBe("lg");
    expect(btn.className).toContain("bg-[var(--pellucid-danger)]");
    expect(btn.className).toContain("h-11");
  });

  test("loading sets aria-busy and disables", () => {
    render(<Button loading>Loading</Button>);
    const btn = screen.getByRole("button");
    expect(btn.getAttribute("aria-busy")).toBe("true");
    expect((btn as HTMLButtonElement).disabled).toBe(true);
  });

  test("explicit disabled wins even if not loading", () => {
    render(<Button disabled>Off</Button>);
    expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(true);
  });

  test("forwards extra className without dropping defaults", () => {
    render(<Button className="custom-x">Compose</Button>);
    const btn = screen.getByRole("button");
    expect(btn.className).toContain("custom-x");
    expect(btn.className).toContain("inline-flex");
  });

  test("asChild renders trigger element", () => {
    render(
      <Button asChild>
        <a href="/x" data-link>
          Link
        </a>
      </Button>,
    );
    const link = screen.getByRole("link", { name: "Link" });
    expect(link.tagName).toBe("A");
    expect(link.getAttribute("data-variant-button")).toBe("solid");
    expect(link.hasAttribute("data-link")).toBe(true);
  });

  test("renders ghost + outline tones", () => {
    render(
      <>
        <Button variant="ghost">Ghost</Button>
        <Button variant="outline">Outline</Button>
      </>,
    );
    expect(
      screen.getByRole("button", { name: "Ghost" }).className,
    ).toContain("hover:bg-[var(--pellucid-surface-raised)]");
    expect(
      screen.getByRole("button", { name: "Outline" }).className,
    ).toContain("border");
  });
});
