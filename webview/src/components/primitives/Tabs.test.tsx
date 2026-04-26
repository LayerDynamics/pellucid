import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { Tabs, TabsContent, TabsList, TabsTrigger } from "./Tabs";

afterEach(() => {
  cleanup();
});

describe("<Tabs>", () => {
  test("default tab is rendered, others hidden", () => {
    render(
      <Tabs defaultValue="a">
        <TabsList>
          <TabsTrigger value="a">A</TabsTrigger>
          <TabsTrigger value="b">B</TabsTrigger>
        </TabsList>
        <TabsContent value="a">Body A</TabsContent>
        <TabsContent value="b">Body B</TabsContent>
      </Tabs>,
    );
    expect(screen.getByText("Body A")).toBeDefined();
    expect(screen.queryByText("Body B")).toBeNull();
  });

  test("clicking trigger swaps content", async () => {
    const user = userEvent.setup();
    render(
      <Tabs defaultValue="a">
        <TabsList>
          <TabsTrigger value="a">A</TabsTrigger>
          <TabsTrigger value="b">B</TabsTrigger>
        </TabsList>
        <TabsContent value="a">Body A</TabsContent>
        <TabsContent value="b">Body B</TabsContent>
      </Tabs>,
    );
    await user.click(screen.getByText("B"));
    expect(screen.queryByText("Body A")).toBeNull();
    expect(screen.getByText("Body B")).toBeDefined();
  });

  test("active trigger receives data-state=active", () => {
    render(
      <Tabs defaultValue="x">
        <TabsList>
          <TabsTrigger value="x">X</TabsTrigger>
          <TabsTrigger value="y">Y</TabsTrigger>
        </TabsList>
        <TabsContent value="x">x</TabsContent>
        <TabsContent value="y">y</TabsContent>
      </Tabs>,
    );
    expect(screen.getByText("X").getAttribute("data-state")).toBe("active");
    expect(screen.getByText("Y").getAttribute("data-state")).toBe("inactive");
  });
});
