import { afterEach, describe, expect, test } from "bun:test";

import {
  loadNewsArticles,
  type ListArticlesOutcome,
  type ListArticlesResponse,
} from "./list";

const SAMPLE: ListArticlesResponse = {
  articles: [
    {
      id: "a1",
      title: "Suez convoy",
      source: "Reuters",
      publishedAtMs: 1_700_000_000_000,
      severity: "high",
    },
  ],
  total: 1,
  stale: false,
};

function jsonResponse(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });
}

afterEach(() => {
  /* nothing global to reset */
});

describe("loadNewsArticles", () => {
  test("200 with valid body → kind=ready", async () => {
    const calls: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      calls.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    const out = await loadNewsArticles({}, { fetchImpl });
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") expect(out.response).toEqual(SAMPLE);
    expect(calls[0]).toBe("/api/news/v1/list-articles");
  });

  test("encodes ?limit + ?severity into query string", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadNewsArticles({ limit: 25, severity: "critical" }, { fetchImpl });
    const url = seen[0] ?? "";
    expect(url).toContain("limit=25");
    expect(url).toContain("severity=critical");
  });

  test("clamps non-finite or fractional limit to floor (>=1)", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadNewsArticles({ limit: 0 }, { fetchImpl });
    const url = seen[0] ?? "";
    expect(url).toContain("limit=1");
  });

  test("network error → kind=error code=network", async () => {
    const fetchImpl: typeof fetch = async () => {
      throw new Error("boom");
    };
    const out = await loadNewsArticles({}, { fetchImpl });
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
            retry_after_secs: 30,
          },
        },
        { "retry-after": "30" },
      );
    const out: ListArticlesOutcome = await loadNewsArticles({}, { fetchImpl });
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
    const out = await loadNewsArticles({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("cache_failure");
  });

  test("unknown error code falls back to 'unknown'", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(500, {
        error: { code: "no_such_code_in_set", message: "x" },
      });
    const out = await loadNewsArticles({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("unknown");
  });

  test("unparseable error body still synthesises an envelope", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json", { status: 500 });
    const out = await loadNewsArticles({}, { fetchImpl });
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
    const out = await loadNewsArticles({}, { fetchImpl });
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
    await loadNewsArticles({}, { fetchImpl, bearerToken: "tok-123" });
    expect(seenAuth).toEqual(["Bearer tok-123"]);
  });

  test("baseUrl prefixes the request URL", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadNewsArticles({}, { fetchImpl, baseUrl: "https://api.example" });
    expect(seen[0]).toBe("https://api.example/api/news/v1/list-articles");
  });

  test("AbortSignal is propagated to fetch", async () => {
    let seenSignal: AbortSignal | undefined;
    const fetchImpl: typeof fetch = async (_input, init) => {
      seenSignal = init?.signal ?? undefined;
      return jsonResponse(200, SAMPLE);
    };
    const ctrl = new AbortController();
    await loadNewsArticles({}, { fetchImpl, signal: ctrl.signal });
    expect(seenSignal).toBe(ctrl.signal);
  });
});
