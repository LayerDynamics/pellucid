import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { Toggle } from "./Toggle";

afterEach(() => {
  cleanup();
});

describe("<Toggle>", () => {
  test("default state is off", () => {
    render(<Toggle aria-label="t">Tog</Toggle>);
    expect(
      screen.getByRole("button", { name: "t" }).getAttribute("data-state"),
    ).toBe("off");
  });

  test("clicking flips state", async () => {
    const user = userEvent.setup();
    render(<Toggle aria-label="bold">B</Toggle>);
    const btn = screen.getByRole("button", { name: "bold" });
    await user.click(btn);
    expect(btn.getAttribute("data-state")).toBe("on");
    await user.click(btn);
    expect(btn.getAttribute("data-state")).toBe("off");
  });

  test("size attribute reflects prop", () => {
    render(
      <>
        <Toggle aria-label="sm" size="sm">
          s
        </Toggle>
        <Toggle aria-label="lg" size="lg">
          l
        </Toggle>
      </>,
    );
    expect(
      screen.getByRole("button", { name: "sm" }).getAttribute("data-size"),
    ).toBe("sm");
    expect(
      screen.getByRole("button", { name: "lg" }).getAttribute("data-size"),
    ).toBe("lg");
  });

  test("controlled pressed prop respected", () => {
    render(
      <Toggle aria-label="ctrl" pressed onPressedChange={() => {}}>
        x
      </Toggle>,
    );
    expect(
      screen.getByRole("button", { name: "ctrl" }).getAttribute("data-state"),
    ).toBe("on");
  });
});
