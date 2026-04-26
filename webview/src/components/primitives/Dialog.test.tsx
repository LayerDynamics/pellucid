import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogTrigger,
} from "./Dialog";

afterEach(() => {
  cleanup();
});

describe("<Dialog>", () => {
  test("trigger opens content portal", async () => {
    const user = userEvent.setup();
    render(
      <Dialog>
        <DialogTrigger>Open</DialogTrigger>
        <DialogContent heading="Hello" blurb="World">
          <DialogClose>Close</DialogClose>
        </DialogContent>
      </Dialog>,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    await user.click(screen.getByText("Open"));
    expect(screen.getByRole("dialog")).toBeDefined();
    expect(screen.getByText("Hello")).toBeDefined();
    expect(screen.getByText("World")).toBeDefined();
  });

  test("close button dismisses dialog", async () => {
    const user = userEvent.setup();
    render(
      <Dialog defaultOpen>
        <DialogContent heading="Title">
          <DialogClose>Close</DialogClose>
        </DialogContent>
      </Dialog>,
    );
    await user.click(screen.getByText("Close"));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  test("attributes Pellucid markers", () => {
    render(
      <Dialog defaultOpen>
        <DialogContent heading="t">x</DialogContent>
      </Dialog>,
    );
    const root = document.querySelector('[data-pellucid="dialog-content"]');
    expect(root).not.toBeNull();
  });
});
