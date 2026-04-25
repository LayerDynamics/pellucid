import { afterEach, describe, expect, test } from "bun:test";

import { useUiStore } from "./useUiStore";

afterEach(() => {
  useUiStore.setState({ theme: "system", lang: "en", sidebarOpen: false, modalStack: [] });
});

describe("useUiStore", () => {
  test("default state", () => {
    const s = useUiStore.getState();
    expect(s.theme).toBe("system");
    expect(s.lang).toBe("en");
    expect(s.sidebarOpen).toBe(false);
    expect(s.modalStack).toEqual([]);
  });

  test("setTheme + setLang persist", () => {
    useUiStore.getState().setTheme("dark");
    useUiStore.getState().setLang("ar");
    const s = useUiStore.getState();
    expect(s.theme).toBe("dark");
    expect(s.lang).toBe("ar");
  });

  test("toggleSidebar flips the boolean", () => {
    useUiStore.getState().toggleSidebar();
    expect(useUiStore.getState().sidebarOpen).toBe(true);
    useUiStore.getState().toggleSidebar();
    expect(useUiStore.getState().sidebarOpen).toBe(false);
  });

  test("setSidebar sets explicit value", () => {
    useUiStore.getState().setSidebar(true);
    expect(useUiStore.getState().sidebarOpen).toBe(true);
  });

  test("modal stack push/top/pop/isOpen", () => {
    useUiStore.getState().pushModal("a");
    useUiStore.getState().pushModal("b");
    expect(useUiStore.getState().topModal()).toBe("b");
    expect(useUiStore.getState().isModalOpen("a")).toBe(true);
    expect(useUiStore.getState().isModalOpen("c")).toBe(false);
    expect(useUiStore.getState().popModal()).toBe("b");
    expect(useUiStore.getState().topModal()).toBe("a");
    expect(useUiStore.getState().popModal()).toBe("a");
    expect(useUiStore.getState().popModal()).toBeNull();
  });
});
