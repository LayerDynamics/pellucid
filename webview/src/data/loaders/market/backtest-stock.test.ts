import { describe, expect, test } from "bun:test";

import {
  loadBacktestStock,
  type BacktestStockOutcome,
  type BacktestStockResponse,
} from "./backtest-stock";

const SAMPLE: BacktestStockResponse = {
  strategies: [
    {
      strategy: "equal-weight",
      picks: [
        { symbol: "SPY", weight: 0.5, percentChange: 0.4, contribution: 0.2 },
        { symbol: "QQQ", weight: 0.5, percentChange: -0.2, contribution: -0.1 },
      ],
      metrics: { totalReturnPct: 0.1, winRatePct: 50, maxDrawdownPct: -0.1 },
    },
  ],
  universe: ["SPY", "QQQ"],
  stale: false,
  assembledAtMs: 1_700_000_000_000,
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

describe("loadBacktestStock", () => {
  test("200 → kind=ready", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    const out = await loadBacktestStock({}, { fetchImpl });
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") expect(out.response).toEqual(SAMPLE);
    expect(seen[0]).toBe("/api/market/v1/backtest-stock");
  });

  test("encodes ?limitUniverse + ?symbols", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadBacktestStock(
      { limitUniverse: 10, symbols: "SPY,QQQ" },
      { fetchImpl },
    );
    const url = seen[0] ?? "";
    expect(url).toContain("limitUniverse=10");
    expect(url).toContain("symbols=SPY%2CQQQ");
  });

  test("clamps non-finite or fractional limitUniverse to floor (>=1)", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadBacktestStock({ limitUniverse: 0.4 }, { fetchImpl });
    expect(seen[0]).toContain("limitUniverse=1");
  });

  test("network error → kind=error code=network", async () => {
    const fetchImpl: typeof fetch = async () => {
      throw new Error("offline");
    };
    const out = await loadBacktestStock({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("network");
      expect(out.message).toContain("offline");
    }
  });

  test("503 outage envelope passes through", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(
        503,
        {
          error: {
            code: "bootstrap_upstream_empty",
            message: "upstream is empty",
          },
        },
        { "retry-after": "30" },
      );
    const out: BacktestStockOutcome = await loadBacktestStock({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("bootstrap_upstream_empty");
      expect(out.retryAfterSecs).toBe(30);
    }
  });

  test("404 empty_universe envelope passes through", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(404, {
        error: { code: "empty_universe", message: "filter excluded all rows" },
      });
    const out = await loadBacktestStock({ symbols: "NVDA" }, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("empty_universe");
  });

  test("unknown error code falls back to 'unknown'", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(500, { error: { code: "no_such_code", message: "x" } });
    const out = await loadBacktestStock({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("unknown");
  });

  test("malformed body in error path still returns synthesised envelope", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json", { status: 500 });
    const out = await loadBacktestStock({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("unknown");
      expect(out.message).toBe("HTTP 500");
    }
  });

  test("malformed 200 body → unknown parse error", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("{not-json", {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    const out = await loadBacktestStock({}, { fetchImpl });
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
      const auth = h.authorization;
      if (typeof auth === "string") seenAuth.push(auth);
      return jsonResponse(200, SAMPLE);
    };
    await loadBacktestStock({}, { fetchImpl, bearerToken: "tier2-tok" });
    expect(seenAuth).toEqual(["Bearer tier2-tok"]);
  });

  test("baseUrl prefixes the URL", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadBacktestStock({}, { fetchImpl, baseUrl: "https://api.example" });
    expect(seen[0]).toBe("https://api.example/api/market/v1/backtest-stock");
  });

  test("AbortSignal propagates", async () => {
    let seen: AbortSignal | undefined;
    const fetchImpl: typeof fetch = async (_input, init) => {
      seen = init?.signal ?? undefined;
      return jsonResponse(200, SAMPLE);
    };
    const ctrl = new AbortController();
    await loadBacktestStock({}, { fetchImpl, signal: ctrl.signal });
    expect(seen).toBe(ctrl.signal);
  });
});
