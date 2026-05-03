import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import {
  fetchBootstrapData,
  getBootstrapHydrationState,
  getHydratedData,
  markBootstrapAsLive,
  resetBootstrapHydrationState,
  type FetchOptions,
} from "./bootstrap";

beforeEach(() => resetBootstrapHydrationState());
afterEach(() => resetBootstrapHydrationState());

/** Build a fetch stub that returns a fixed response. */
function fixedFetch(
  status: number,
  body: unknown,
  delayMs = 0,
): typeof fetch {
  return (async (_input: unknown, init?: { signal?: AbortSignal }) => {
    if (delayMs > 0) {
      await new Promise<void>((resolve, reject) => {
        const t = setTimeout(resolve, delayMs);
        init?.signal?.addEventListener("abort", () => {
          clearTimeout(t);
          const err = new Error("aborted");
          err.name = "AbortError";
          reject(err);
        });
      });
    }
    return new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  }) as typeof fetch;
}

describe("getBootstrapHydrationState (initial)", () => {
  test("starts with no fast, no slow, not live", () => {
    const s = getBootstrapHydrationState();
    expect(s.hasFast).toBe(false);
    expect(s.hasSlow).toBe(false);
    expect(s.isLive).toBe(false);
    expect(s.fastOutcome).toBeNull();
    expect(s.slowOutcome).toBeNull();
  });

  test("returned snapshot is a shallow copy (no mutation leak)", () => {
    const s1 = getBootstrapHydrationState();
    s1.hasFast = true; // mutate the snapshot
    const s2 = getBootstrapHydrationState();
    expect(s2.hasFast).toBe(false); // singleton untouched
  });
});

describe("markBootstrapAsLive", () => {
  test("flips isLive without touching tier state", () => {
    markBootstrapAsLive();
    const s = getBootstrapHydrationState();
    expect(s.isLive).toBe(true);
    expect(s.hasFast).toBe(false);
    expect(s.hasSlow).toBe(false);
  });
});

describe("fetchBootstrapData — fast tier 200", () => {
  test("populates fastOutcome and flips hasFast", async () => {
    const opts: FetchOptions = {
      fetchImpl: fixedFetch(200, {
        data: { "news:breaking:v1": { title: "hi" } },
        missing: ["aviation:active-notams:v1"],
        negative: [],
      }),
    };
    const out = await fetchBootstrapData("fast", opts);
    expect(out.status).toBe(200);
    expect(out.data["news:breaking:v1"]).toEqual({ title: "hi" });
    expect(out.missing).toEqual(["aviation:active-notams:v1"]);
    expect(out.negative).toEqual([]);
    expect(out.errorCode).toBeNull();

    const s = getBootstrapHydrationState();
    expect(s.hasFast).toBe(true);
    expect(s.hasSlow).toBe(false);
    expect(s.fastOutcome?.status).toBe(200);
  });

  test("getHydratedData reads from fastOutcome data", async () => {
    await fetchBootstrapData("fast", {
      fetchImpl: fixedFetch(200, {
        data: { "k": { v: 42 } },
        missing: [],
        negative: [],
      }),
    });
    expect(getHydratedData<{ v: number }>("k")?.v).toBe(42);
  });
});

describe("fetchBootstrapData — slow tier", () => {
  test("populates slowOutcome separately from fast", async () => {
    await fetchBootstrapData("slow", {
      fetchImpl: fixedFetch(200, {
        data: { "k1": "v1" },
        missing: [],
        negative: [],
      }),
    });
    const s = getBootstrapHydrationState();
    expect(s.hasSlow).toBe(true);
    expect(s.hasFast).toBe(false);
    expect(s.slowOutcome?.data["k1"]).toBe("v1");
  });

  test("getHydratedData falls through to slow when fast lacks the key", async () => {
    await fetchBootstrapData("fast", {
      fetchImpl: fixedFetch(200, {
        data: { "fast-only": "F" },
        missing: [],
        negative: [],
      }),
    });
    await fetchBootstrapData("slow", {
      fetchImpl: fixedFetch(200, {
        data: { "slow-only": "S" },
        missing: [],
        negative: [],
      }),
    });
    expect(getHydratedData<string>("fast-only")).toBe("F");
    expect(getHydratedData<string>("slow-only")).toBe("S");
    expect(getHydratedData("absent")).toBeUndefined();
  });
});

describe("fetchBootstrapData — M4 503 outage path", () => {
  test("captures errorCode + retryAfterSecs without throwing", async () => {
    const out = await fetchBootstrapData("fast", {
      fetchImpl: fixedFetch(503, {
        error: {
          code: "bootstrap_upstream_empty",
          message: "outage",
          retry_after_secs: 30,
          requested: 67,
        },
      }),
    });
    expect(out.status).toBe(503);
    expect(out.errorCode).toBe("bootstrap_upstream_empty");
    expect(out.retryAfterSecs).toBe(30);
    expect(out.data).toEqual({});

    const s = getBootstrapHydrationState();
    expect(s.hasFast).toBe(false); // 503 must NOT set hasFast
    expect(s.fastOutcome?.status).toBe(503);
    expect(s.fastOutcome?.errorCode).toBe("bootstrap_upstream_empty");
    expect(s.fastOutcome?.retryAfterSecs).toBe(30);
  });
});

describe("fetchBootstrapData — budget exceeded", () => {
  test("aborts the request and surfaces budgetExceeded=true", async () => {
    const out = await fetchBootstrapData("fast", {
      fetchImpl: fixedFetch(200, {}, 200), // upstream takes 200ms
      budgetMs: 50, // budget is 50ms — abort fires first
    });
    expect(out.status).toBe(0);
    expect(out.errorCode).toBe("abort");
    expect(out.budgetExceeded).toBe(true);
  });
});

describe("fetchBootstrapData — non-200 non-503", () => {
  test("400 invalid-request envelope captured without throwing", async () => {
    const out = await fetchBootstrapData("fast", {
      fetchImpl: fixedFetch(400, {
        error: { code: "invalid_request", message: "bad tier" },
      }),
    });
    expect(out.status).toBe(400);
    expect(out.errorCode).toBe("invalid_request");
    expect(out.retryAfterSecs).toBeNull();

    const s = getBootstrapHydrationState();
    expect(s.hasFast).toBe(false);
  });
});

describe("getHydratedData", () => {
  test("returns undefined when neither outcome is present", () => {
    expect(getHydratedData("never:set:v1")).toBeUndefined();
  });
});
