import { afterEach, describe, expect, test } from "bun:test";

import {
  loadFearGreed,
  type FearGreedOutcome,
  type FearGreedResponse,
} from "./fear-greed";

const SAMPLE: FearGreedResponse = {
  score: 62,
  label: "greed",
  components: [
    { name: "volatility", score: 75, label: "greed", rationale: "VIX at 14.2" },
    { name: "momentum", score: 60, label: "greed", rationale: "5/8 advancing" },
    { name: "strength", score: 55, label: "neutral", rationale: "Avg +0.18%" },
    { name: "volume", score: 58, label: "greed", rationale: "ETF activity 1.18×" },
  ],
  assembledAtMs: 1_700_000_000_000,
  stale: false,
};

function jsonResponse(
  status: number,
  body: unknown,
  headers: Record<string, string> = {},
): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });
}

afterEach(() => {});

describe("loadFearGreed", () => {
  test("200 with valid body → kind=ready", async () => {
    const calls: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      calls.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    const out = await loadFearGreed({ fetchImpl });
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") expect(out.response).toEqual(SAMPLE);
    expect(calls[0]).toBe("/api/market/v1/fear-greed");
  });

  test("network error → kind=error code=network", async () => {
    const fetchImpl: typeof fetch = async () => {
      throw new Error("boom");
    };
    const out = await loadFearGreed({ fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("network");
      expect(out.httpStatus).toBe(0);
    }
  });

  test("503 with bootstrap_upstream_empty body + retry-after header", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(
        503,
        { error: { code: "bootstrap_upstream_empty", message: "x" } },
        { "retry-after": "30" },
      );
    const out: FearGreedOutcome = await loadFearGreed({ fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("bootstrap_upstream_empty");
      expect(out.retryAfterSecs).toBe(30);
    }
  });

  test("502 cache_failure passes through", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(502, {
        error: { code: "cache_failure", message: "db locked" },
      });
    const out = await loadFearGreed({ fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("cache_failure");
  });

  test("unknown error code falls back to 'unknown'", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(500, { error: { code: "no_such", message: "x" } });
    const out = await loadFearGreed({ fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("unknown");
  });

  test("unparseable error body still synthesises an envelope", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json", { status: 500 });
    const out = await loadFearGreed({ fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("unknown");
      expect(out.message).toBe("HTTP 500");
    }
  });

  test("200 with malformed body → kind=error code=unknown", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json", {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    const out = await loadFearGreed({ fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("unknown");
      expect(out.message).toContain("parse:");
    }
  });

  test("bearer token forwarded in Authorization header", async () => {
    const seenAuth: string[] = [];
    const fetchImpl: typeof fetch = async (_input, init) => {
      const h = (init?.headers ?? {}) as Record<string, string>;
      if (typeof h.authorization === "string") seenAuth.push(h.authorization);
      return jsonResponse(200, SAMPLE);
    };
    await loadFearGreed({ fetchImpl, bearerToken: "tok-1" });
    expect(seenAuth).toEqual(["Bearer tok-1"]);
  });

  test("baseUrl prefixes the request URL", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadFearGreed({ fetchImpl, baseUrl: "https://api.example" });
    expect(seen[0]).toBe("https://api.example/api/market/v1/fear-greed");
  });

  test("AbortSignal is propagated to fetch", async () => {
    let seenSignal: AbortSignal | undefined;
    const fetchImpl: typeof fetch = async (_input, init) => {
      seenSignal = init?.signal ?? undefined;
      return jsonResponse(200, SAMPLE);
    };
    const ctrl = new AbortController();
    await loadFearGreed({ fetchImpl, signal: ctrl.signal });
    expect(seenSignal).toBe(ctrl.signal);
  });
});
