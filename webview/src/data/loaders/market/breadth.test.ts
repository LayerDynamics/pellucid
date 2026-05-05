import { describe, expect, test } from "bun:test";

import {
  loadBreadth,
  type BreadthOutcome,
  type BreadthResponse,
} from "./breadth";

const SAMPLE: BreadthResponse = {
  advancers: 5,
  decliners: 2,
  unchanged: 1,
  advanceDeclineLine: 3,
  newHighs: 1,
  newLows: 0,
  topAdvancers: [{ symbol: "WIN", percentChange: 5.0 }],
  topDecliners: [{ symbol: "LOSE", percentChange: -2.0 }],
  universe: 8,
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

describe("loadBreadth", () => {
  test("200 → kind=ready", async () => {
    const fetchImpl: typeof fetch = async () => jsonResponse(200, SAMPLE);
    const out = await loadBreadth({}, { fetchImpl });
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") expect(out.response).toEqual(SAMPLE);
  });

  test("encodes ?topN", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadBreadth({ topN: 10 }, { fetchImpl });
    expect(seen[0]).toContain("topN=10");
  });

  test("clamps fractional topN to floor (>=1)", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadBreadth({ topN: 0.5 }, { fetchImpl });
    expect(seen[0]).toContain("topN=1");
  });

  test("network error → code=network", async () => {
    const fetchImpl: typeof fetch = async () => {
      throw new Error("offline");
    };
    const out = await loadBreadth({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("network");
  });

  test("503 outage envelope passes through", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(
        503,
        { error: { code: "bootstrap_upstream_empty", message: "x" } },
        { "retry-after": "30" },
      );
    const out: BreadthOutcome = await loadBreadth({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("bootstrap_upstream_empty");
      expect(out.retryAfterSecs).toBe(30);
    }
  });

  test("502 cache_failure passes through", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(502, { error: { code: "cache_failure", message: "x" } });
    const out = await loadBreadth({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("cache_failure");
  });

  test("unknown error code falls back to 'unknown'", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(500, { error: { code: "no_such_code", message: "x" } });
    const out = await loadBreadth({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("unknown");
  });

  test("malformed body in error path still synthesises envelope", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json", { status: 500 });
    const out = await loadBreadth({}, { fetchImpl });
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
    const out = await loadBreadth({}, { fetchImpl });
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
    await loadBreadth({}, { fetchImpl, baseUrl: "https://api.example" });
    expect(seen[0]).toBe("https://api.example/api/market/v1/breadth");
  });

  test("bearer token forwarded", async () => {
    const seenAuth: string[] = [];
    const fetchImpl: typeof fetch = async (_input, init) => {
      const h = (init?.headers ?? {}) as Record<string, string>;
      const auth = h.authorization;
      if (typeof auth === "string") seenAuth.push(auth);
      return jsonResponse(200, SAMPLE);
    };
    await loadBreadth({}, { fetchImpl, bearerToken: "tok" });
    expect(seenAuth).toEqual(["Bearer tok"]);
  });

  test("AbortSignal propagates", async () => {
    let seen: AbortSignal | undefined;
    const fetchImpl: typeof fetch = async (_input, init) => {
      seen = init?.signal ?? undefined;
      return jsonResponse(200, SAMPLE);
    };
    const ctrl = new AbortController();
    await loadBreadth({}, { fetchImpl, signal: ctrl.signal });
    expect(seen).toBe(ctrl.signal);
  });
});
