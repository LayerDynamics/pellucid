import { afterEach, describe, expect, test } from "bun:test";

import {
  loadGdeltFeed,
  parseGdeltSeenDate,
  type GdeltFeedOutcome,
  type GdeltFeedResponse,
} from "./gdelt";

const SAMPLE: GdeltFeedResponse = {
  rows: [
    {
      url: "https://a.example/1",
      title: "Title 1",
      seenDate: "20260504T120000Z",
      socialImage: "",
      domain: "a.example",
      language: "English",
      sourceCountry: "Iran",
    },
  ],
  query: "(theme:KILL)",
  timespan: "24h",
  assembledAtMs: 1_700_000_000_000,
  total: 1,
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

describe("loadGdeltFeed", () => {
  test("200 with valid body → kind=ready", async () => {
    const calls: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      calls.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    const out = await loadGdeltFeed({}, { fetchImpl });
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") expect(out.response).toEqual(SAMPLE);
    expect(calls[0]).toBe("/api/intelligence/v1/gdelt-feed");
  });

  test("encodes ?limit + ?country into the query string", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadGdeltFeed({ limit: 25, country: "Iran" }, { fetchImpl });
    const url = seen[0] ?? "";
    expect(url).toContain("limit=25");
    expect(url).toContain("country=Iran");
  });

  test("trims whitespace + drops empty country", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadGdeltFeed({ country: "  Iran  " }, { fetchImpl });
    expect(seen[0] ?? "").toContain("country=Iran");
    seen.length = 0;
    await loadGdeltFeed({ country: "   " }, { fetchImpl });
    expect(seen[0] ?? "").toBe("/api/intelligence/v1/gdelt-feed");
  });

  test("clamps non-finite or fractional limit to floor (>=1)", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadGdeltFeed({ limit: 0 }, { fetchImpl });
    expect(seen[0] ?? "").toContain("limit=1");
  });

  test("network error → kind=error code=network", async () => {
    const fetchImpl: typeof fetch = async () => {
      throw new Error("boom");
    };
    const out = await loadGdeltFeed({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("network");
      expect(out.httpStatus).toBe(0);
      expect(out.message).toContain("boom");
    }
  });

  test("503 with bootstrap_upstream_empty body + retry-after header", async () => {
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
    const out: GdeltFeedOutcome = await loadGdeltFeed({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("bootstrap_upstream_empty");
      expect(out.httpStatus).toBe(503);
      expect(out.retryAfterSecs).toBe(30);
    }
  });

  test("502 cache_failure passes through", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(502, {
        error: { code: "cache_failure", message: "db locked" },
      });
    const out = await loadGdeltFeed({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("cache_failure");
  });

  test("unknown error code falls back to 'unknown'", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(500, {
        error: { code: "no_such_code_in_set", message: "x" },
      });
    const out = await loadGdeltFeed({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("unknown");
  });

  test("unparseable error body still synthesises an envelope", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json", { status: 500 });
    const out = await loadGdeltFeed({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("unknown");
      expect(out.message).toBe("HTTP 500");
    }
  });

  test("200 with malformed body → kind=error code=unknown", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json at all", {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    const out = await loadGdeltFeed({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("unknown");
      expect(out.httpStatus).toBe(200);
      expect(out.message).toContain("parse:");
    }
  });

  test("bearer token forwarded in Authorization header", async () => {
    const seenAuth: string[] = [];
    const fetchImpl: typeof fetch = async (_input, init) => {
      const h = (init?.headers ?? {}) as Record<string, string>;
      const value = h.authorization;
      if (typeof value === "string") seenAuth.push(value);
      return jsonResponse(200, SAMPLE);
    };
    await loadGdeltFeed({}, { fetchImpl, bearerToken: "tok-123" });
    expect(seenAuth).toEqual(["Bearer tok-123"]);
  });

  test("baseUrl prefixes the request URL", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadGdeltFeed({}, { fetchImpl, baseUrl: "https://api.example" });
    expect(seen[0]).toBe("https://api.example/api/intelligence/v1/gdelt-feed");
  });

  test("AbortSignal is propagated to fetch", async () => {
    let seenSignal: AbortSignal | undefined;
    const fetchImpl: typeof fetch = async (_input, init) => {
      seenSignal = init?.signal ?? undefined;
      return jsonResponse(200, SAMPLE);
    };
    const ctrl = new AbortController();
    await loadGdeltFeed({}, { fetchImpl, signal: ctrl.signal });
    expect(seenSignal).toBe(ctrl.signal);
  });
});

describe("parseGdeltSeenDate", () => {
  test("parses a well-formed YYYYMMDDTHHMMSSZ string into UTC ms", () => {
    const ms = parseGdeltSeenDate("20260504T120000Z");
    expect(ms).toBe(Date.UTC(2026, 4, 4, 12, 0, 0));
  });

  test("returns null on malformed input (missing Z)", () => {
    expect(parseGdeltSeenDate("20260504T120000")).toBeNull();
  });

  test("returns null on empty string", () => {
    expect(parseGdeltSeenDate("")).toBeNull();
  });

  test("returns null on a different format", () => {
    expect(parseGdeltSeenDate("2026-05-04T12:00:00Z")).toBeNull();
  });
});
