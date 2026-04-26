/**
 * H1 rotation cadence — slow desktop e2e.
 *
 * Watches the live `__TAURI__` event stream for `token_rotated` events
 * over a 35-minute window. Six events are expected (one every ~5 min).
 * Tagged `@slow` so it only runs on the dedicated nightly CI shard;
 * see `.github/workflows/desktop-slow.yml`.
 *
 * Companion to the Rust H1 regression test in
 * `crates/pellucid-tauri/tests/regression_h1.rs`. The Rust test proves
 * the rotation logic in isolation; this spec proves the production
 * wiring (Tauri event emit, webview listener, vault persistence) works
 * end-to-end against the real desktop binary.
 */

import { browser, expect } from "@wdio/globals";

const ROTATION_INTERVAL_MS = 5 * 60 * 1_000;
const REQUIRED_ROTATIONS = 6;
const TOTAL_WINDOW_MS = ROTATION_INTERVAL_MS * (REQUIRED_ROTATIONS + 1); // 35 min

interface RotationCapture {
  observed: { atMs: number; token: string }[];
}

declare global {
  interface Window {
    __pellucidH1Capture?: RotationCapture;
  }
}

describe("@desktop @slow pellucid-tauri token rotation cadence", () => {
  it("emits at least 6 token_rotated events over 35 minutes", async () => {
    await browser.url("/");
    const supported = await browser.execute(() => {
      type TauriBridge = {
        invoke: (cmd: string) => Promise<string | null>;
        event?: {
          listen: (
            name: string,
            cb: (payload: { payload: { at_ms: number } }) => void,
          ) => Promise<() => void>;
        };
      };
      const tauri = (window as unknown as { __TAURI__?: TauriBridge })
        .__TAURI__;
      if (!tauri || !tauri.event) {
        return false;
      }
      window.__pellucidH1Capture = { observed: [] };
      const listener = tauri.event.listen(
        "token_rotated",
        async (event: { payload: { at_ms: number } }) => {
          const tok = await tauri.invoke("get_local_api_token");
          window.__pellucidH1Capture!.observed.push({
            atMs: event.payload.at_ms,
            token: typeof tok === "string" ? tok : "",
          });
        },
      );
      void listener; // listener kept alive for test duration
      return true;
    });

    if (!supported) {
      // Tauri runtime not present (web shard) — soft-skip per the
      // task plan; the Rust regression test covers logic.
      return;
    }

    await browser.waitUntil(
      async () => {
        const count = await browser.execute(
          () => window.__pellucidH1Capture?.observed.length ?? 0,
        );
        return count >= REQUIRED_ROTATIONS;
      },
      {
        timeout: TOTAL_WINDOW_MS,
        interval: 30_000,
        timeoutMsg: `expected ≥ ${REQUIRED_ROTATIONS} rotations within ${TOTAL_WINDOW_MS} ms`,
      },
    );

    const log = await browser.execute(
      () => window.__pellucidH1Capture?.observed ?? [],
    );
    expect(log.length).toBeGreaterThanOrEqual(REQUIRED_ROTATIONS);

    // Every captured token must be unique — duplicates would mean the
    // rotator is recycling values, which violates SPEC-001 §10.3.
    const distinctTokens = new Set(log.map((e) => e.token));
    expect(distinctTokens.size).toBe(log.length);

    // Adjacent rotations must be no closer than the configured
    // interval minus a small jitter tolerance, and no further than
    // the interval plus a generous wakeup buffer.
    for (let i = 1; i < log.length; i++) {
      const delta = log[i].atMs - log[i - 1].atMs;
      expect(delta).toBeGreaterThan(ROTATION_INTERVAL_MS - 5_000);
      expect(delta).toBeLessThan(ROTATION_INTERVAL_MS + 90_000);
    }
  });
});
