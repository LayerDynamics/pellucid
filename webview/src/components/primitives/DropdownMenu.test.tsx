import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "./DropdownMenu";

afterEach(() => {
  cleanup();
});

describe("<DropdownMenu>", () => {
  test("trigger opens menu and reveals items", async () => {
    const user = userEvent.setup();
    render(
      <DropdownMenu>
        <DropdownMenuTrigger>Menu</DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuLabel>Group</DropdownMenuLabel>
          <DropdownMenuItem>Item A</DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem>Item B</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>,
    );
    expect(screen.queryByText("Item A")).toBeNull();
    await user.click(screen.getByText("Menu"));
    expect(screen.getByText("Item A")).toBeDefined();
    expect(screen.getByText("Item B")).toBeDefined();
    expect(screen.getByText("Group")).toBeDefined();
  });

  test("disabled items expose pointer-events:none style", async () => {
    const user = userEvent.setup();
    render(
      <DropdownMenu>
        <DropdownMenuTrigger>Open</DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem disabled>Locked</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>,
    );
    await user.click(screen.getByText("Open"));
    const item = screen.getByText("Locked");
    expect(item.getAttribute("data-disabled")).not.toBeNull();
  });
});
