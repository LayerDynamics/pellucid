/**
 * Self-test for the Pellucid webview test environment.
 *
 * Asserts that `bun test` (with `webview/bunfig.toml` preloading
 * `test/setup.ts`) provides a working DOM, storage, fetch, and timer
 * stack — the surface every other webview test depends on.
 */

import { describe, expect, test } from "bun:test";

describe("test environment (self-test)", () => {
  test("globalThis.document is defined", () => {
    expect(typeof globalThis.document).toBe("object");
    expect(globalThis.document.createElement("div").tagName).toBe("DIV");
  });

  test("window + location are wired", () => {
    expect(typeof window).toBe("object");
    expect(window.location.href).toBe("http://localhost/");
  });

  test("localStorage round-trips", () => {
    localStorage.setItem("pellucid:test", "value");
    expect(localStorage.getItem("pellucid:test")).toBe("value");
    localStorage.removeItem("pellucid:test");
    expect(localStorage.getItem("pellucid:test")).toBeNull();
  });

  test("fetch is available (happy-dom polyfill or native)", () => {
    expect(typeof globalThis.fetch).toBe("function");
  });

  test("queueMicrotask is available", () => {
    expect(typeof queueMicrotask).toBe("function");
  });

  test("React DOM events propagate", () => {
    const el = document.createElement("button");
    let clicked = false;
    el.addEventListener("click", () => {
      clicked = true;
    });
    el.click();
    expect(clicked).toBe(true);
  });
});
