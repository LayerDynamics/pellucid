import { afterEach, describe, expect, test } from "bun:test";

import {
  loadAnalyzeStock,
  type AnalyzeStockOutcome,
  type AnalyzeStockResponse,
} from "./analyze-stock";

const SAMPLE: AnalyzeStockResponse = {
  symbol: "SPY",
  price: 524,
  previousClose: 522,
  currency: "USD",
  exchange: "PCX",
  regularMarketTimeMs: 1_714_060_800_000,
  metrics: {
    dollarChange: 2,
    percentChange: 0.383,
    trend: "up",
    magnitude: "small",
    rangePosition: 0.5,
  },
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

describe("loadAnalyzeStock", () => {
  test("missing symbol → invalid_request without a network call", async () => {
    let called = false;
    const fetchImpl: typeof fetch = async () => {
      called = true;
      return jsonResponse(200, SAMPLE);
    };
    const out = await loadAnalyzeStock({ symbol: "" }, { fetchImpl });
    expect(called).toBe(false);
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("invalid_request");
  });

  test("whitespace-only symbol → invalid_request", async () => {
    const out = await loadAnalyzeStock(
      { symbol: "  " },
      { fetchImpl: async () => jsonResponse(200, SAMPLE) },
    );
    expect(out.kind).toBe("error");
  });

  test("200 with valid body → kind=ready", async () => {
    const calls: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      calls.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    const out = await loadAnalyzeStock({ symbol: "SPY" }, { fetchImpl });
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") expect(out.response).toEqual(SAMPLE);
    expect(calls[0]).toBe("/api/market/v1/analyze-stock?symbol=SPY");
  });

  test("uppercases + trims the symbol before encoding", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadAnalyzeStock({ symbol: "  spy  " }, { fetchImpl });
    expect(seen[0] ?? "").toContain("symbol=SPY");
  });

  test("network error → kind=error code=network", async () => {
    const fetchImpl: typeof fetch = async () => {
      throw new Error("boom");
    };
    const out = await loadAnalyzeStock({ symbol: "SPY" }, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("network");
  });

  test("404 symbol_not_found passes through", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(404, {
        error: { code: "symbol_not_found", message: "x" },
      });
    const out = await loadAnalyzeStock({ symbol: "NVDA" }, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("symbol_not_found");
      expect(out.httpStatus).toBe(404);
    }
  });

  test("403 entitlement_forbidden passes through (gateway tier gate)", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(403, {
        error: { code: "entitlement_forbidden", message: "upgrade" },
      });
    const out = await loadAnalyzeStock({ symbol: "SPY" }, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("entitlement_forbidden");
  });

  test("503 entitlement_upstream_down → retry-after surfaced", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(
        503,
        { error: { code: "entitlement_upstream_down", message: "x" } },
        { "retry-after": "30" },
      );
    const out: AnalyzeStockOutcome = await loadAnalyzeStock(
      { symbol: "SPY" },
      { fetchImpl },
    );
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("entitlement_upstream_down");
      expect(out.retryAfterSecs).toBe(30);
    }
  });

  test("503 bootstrap_upstream_empty → retry-after surfaced", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(
        503,
        { error: { code: "bootstrap_upstream_empty", message: "x" } },
        { "retry-after": "30" },
      );
    const out = await loadAnalyzeStock({ symbol: "SPY" }, { fetchImpl });
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
    const out = await loadAnalyzeStock({ symbol: "SPY" }, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("cache_failure");
  });

  test("unknown error code falls back to 'unknown'", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(500, { error: { code: "no_such", message: "x" } });
    const out = await loadAnalyzeStock({ symbol: "SPY" }, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("unknown");
  });

  test("unparseable error body still synthesises an envelope", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json", { status: 500 });
    const out = await loadAnalyzeStock({ symbol: "SPY" }, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.message).toBe("HTTP 500");
  });

  test("200 with malformed body → kind=error code=unknown", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json", {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    const out = await loadAnalyzeStock({ symbol: "SPY" }, { fetchImpl });
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
    await loadAnalyzeStock(
      { symbol: "SPY" },
      { fetchImpl, bearerToken: "tok-1" },
    );
    expect(seenAuth).toEqual(["Bearer tok-1"]);
  });

  test("baseUrl prefixes the request URL", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadAnalyzeStock(
      { symbol: "SPY" },
      { fetchImpl, baseUrl: "https://api.example" },
    );
    expect(seen[0]).toBe(
      "https://api.example/api/market/v1/analyze-stock?symbol=SPY",
    );
  });

  test("AbortSignal is propagated to fetch", async () => {
    let seenSignal: AbortSignal | undefined;
    const fetchImpl: typeof fetch = async (_input, init) => {
      seenSignal = init?.signal ?? undefined;
      return jsonResponse(200, SAMPLE);
    };
    const ctrl = new AbortController();
    await loadAnalyzeStock(
      { symbol: "SPY" },
      { fetchImpl, signal: ctrl.signal },
    );
    expect(seenSignal).toBe(ctrl.signal);
  });
});
