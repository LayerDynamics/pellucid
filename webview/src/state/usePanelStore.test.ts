import { afterEach, describe, expect, test } from "bun:test";

import { usePanelStore } from "./usePanelStore";

afterEach(() => usePanelStore.getState().reset());

describe("usePanelStore", () => {
  test("setLayout creates a default layout when missing", () => {
    usePanelStore.getState().setLayout("aviation", { rowSpan: 2 });
    const layout = usePanelStore.getState().getLayout("aviation");
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(1);
  });

  test("setLayout merges over existing", () => {
    usePanelStore.getState().setLayout("p", { rowSpan: 1, colSpan: 1 });
    usePanelStore.getState().setLayout("p", { rowSpan: 4 });
    const layout = usePanelStore.getState().getLayout("p");
    expect(layout?.rowSpan).toBe(4);
    expect(layout?.colSpan).toBe(1);
  });

  test("hide / show toggle the hidden set", () => {
    usePanelStore.getState().hide("p");
    expect(usePanelStore.getState().isHidden("p")).toBe(true);
    usePanelStore.getState().show("p");
    expect(usePanelStore.getState().isHidden("p")).toBe(false);
  });

  test("registered() lists every known panel", () => {
    usePanelStore.getState().setLayout("a", {});
    usePanelStore.getState().setLayout("b", {});
    expect(usePanelStore.getState().registered().sort()).toEqual(["a", "b"]);
  });

  test("reset() clears layouts and hidden set", () => {
    usePanelStore.getState().setLayout("a", {});
    usePanelStore.getState().hide("b");
    usePanelStore.getState().reset();
    expect(usePanelStore.getState().registered()).toEqual([]);
    expect(usePanelStore.getState().isHidden("b")).toBe(false);
  });
});
