import { describe, expect, test } from "bun:test";

import {
  loadEtfFlows,
  type EtfFlowsOutcome,
  type EtfFlowsResponse,
} from "./etf-flows";

const SAMPLE: EtfFlowsResponse = {
  rows: [
    {
      symbol: "SPY",
      latestDollarVolume: 1_000_000,
      avgDollarVolume: 500_000,
      activityRatio: 2.0,
      latestSessionTs: 1_700_000_000,
    },
  ],
  lookbackDays: 5,
  total: 1,
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

describe("loadEtfFlows", () => {
  test("200 → kind=ready", async () => {
    const fetchImpl: typeof fetch = async () => jsonResponse(200, SAMPLE);
    const out = await loadEtfFlows({}, { fetchImpl });
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") expect(out.response).toEqual(SAMPLE);
  });

  test("encodes ?limit + ?sort + ?symbols", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadEtfFlows(
      { limit: 25, sort: "dollar-volume-desc", symbols: "SPY,QQQ" },
      { fetchImpl },
    );
    const url = seen[0] ?? "";
    expect(url).toContain("limit=25");
    expect(url).toContain("sort=dollar-volume-desc");
    expect(url).toContain("symbols=SPY%2CQQQ");
  });

  test("clamps fractional limit to floor (>=1)", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadEtfFlows({ limit: 0.5 }, { fetchImpl });
    expect(seen[0]).toContain("limit=1");
  });

  test("network error → code=network", async () => {
    const fetchImpl: typeof fetch = async () => {
      throw new Error("offline");
    };
    const out = await loadEtfFlows({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("network");
  });

  test("400 invalid_request passes through", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(400, {
        error: { code: "invalid_request", message: "sort bogus" },
      });
    const out: EtfFlowsOutcome = await loadEtfFlows(
      { sort: "activity-ratio-desc" },
      { fetchImpl },
    );
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("invalid_request");
  });

  test("503 outage envelope passes through", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(
        503,
        { error: { code: "bootstrap_upstream_empty", message: "x" } },
        { "retry-after": "30" },
      );
    const out = await loadEtfFlows({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("bootstrap_upstream_empty");
      expect(out.retryAfterSecs).toBe(30);
    }
  });

  test("unknown error code falls back to 'unknown'", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(500, { error: { code: "no_such_code", message: "x" } });
    const out = await loadEtfFlows({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("unknown");
  });

  test("malformed body in error path still synthesises envelope", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json", { status: 500 });
    const out = await loadEtfFlows({}, { fetchImpl });
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
    const out = await loadEtfFlows({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("unknown");
      expect(out.message).toContain("parse:");
    }
  });

  test("baseUrl prefixes the URL", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadEtfFlows({}, { fetchImpl, baseUrl: "https://api.example" });
    expect(seen[0]).toBe("https://api.example/api/market/v1/etf-flows");
  });

  test("bearer token forwarded", async () => {
    const seenAuth: string[] = [];
    const fetchImpl: typeof fetch = async (_input, init) => {
      const h = (init?.headers ?? {}) as Record<string, string>;
      const auth = h.authorization;
      if (typeof auth === "string") seenAuth.push(auth);
      return jsonResponse(200, SAMPLE);
    };
    await loadEtfFlows({}, { fetchImpl, bearerToken: "tok" });
    expect(seenAuth).toEqual(["Bearer tok"]);
  });

  test("AbortSignal propagates", async () => {
    let seen: AbortSignal | undefined;
    const fetchImpl: typeof fetch = async (_input, init) => {
      seen = init?.signal ?? undefined;
      return jsonResponse(200, SAMPLE);
    };
    const ctrl = new AbortController();
    await loadEtfFlows({}, { fetchImpl, signal: ctrl.signal });
    expect(seen).toBe(ctrl.signal);
  });
});
