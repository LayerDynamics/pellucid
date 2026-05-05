import { afterEach, describe, expect, test } from "bun:test";

import {
  loadTelegramFeed,
  parseIsoDatetime,
  type FeedOutcome,
  type FeedResponse,
} from "./telegram";

const SAMPLE: FeedResponse = {
  rows: [
    {
      channel: "rt_intl_news",
      dataPost: "rt_intl_news/12345",
      url: "https://t.me/rt_intl_news/12345",
      datetime: "2026-05-04T12:00:00Z",
      text: "body",
      views: "1.2K",
    },
  ],
  channels: ["rt_intl_news", "isw_warstudies"],
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

describe("loadTelegramFeed", () => {
  test("200 with valid body → kind=ready", async () => {
    const calls: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      calls.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    const out = await loadTelegramFeed({}, { fetchImpl });
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") expect(out.response).toEqual(SAMPLE);
    expect(calls[0]).toBe("/api/telegram/v1/feed");
  });

  test("encodes ?limit + ?channel into the query string", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadTelegramFeed(
      { limit: 25, channel: "rt_intl_news" },
      { fetchImpl },
    );
    const url = seen[0] ?? "";
    expect(url).toContain("limit=25");
    expect(url).toContain("channel=rt_intl_news");
  });

  test("trims whitespace + drops empty channel", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadTelegramFeed({ channel: "   " }, { fetchImpl });
    expect(seen[0] ?? "").toBe("/api/telegram/v1/feed");
  });

  test("clamps non-finite or fractional limit to floor (>=1)", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadTelegramFeed({ limit: 0 }, { fetchImpl });
    expect(seen[0] ?? "").toContain("limit=1");
  });

  test("network error → kind=error code=network", async () => {
    const fetchImpl: typeof fetch = async () => {
      throw new Error("boom");
    };
    const out = await loadTelegramFeed({}, { fetchImpl });
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
        {
          error: {
            code: "bootstrap_upstream_empty",
            message: "upstream is empty",
          },
        },
        { "retry-after": "30" },
      );
    const out: FeedOutcome = await loadTelegramFeed({}, { fetchImpl });
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
    const out = await loadTelegramFeed({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("cache_failure");
  });

  test("unknown error code falls back to 'unknown'", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(500, {
        error: { code: "no_such_code_in_set", message: "x" },
      });
    const out = await loadTelegramFeed({}, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("unknown");
  });

  test("unparseable error body still synthesises an envelope", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json", { status: 500 });
    const out = await loadTelegramFeed({}, { fetchImpl });
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
    const out = await loadTelegramFeed({}, { fetchImpl });
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
      const value = h.authorization;
      if (typeof value === "string") seenAuth.push(value);
      return jsonResponse(200, SAMPLE);
    };
    await loadTelegramFeed({}, { fetchImpl, bearerToken: "tok-1" });
    expect(seenAuth).toEqual(["Bearer tok-1"]);
  });

  test("baseUrl prefixes the request URL", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadTelegramFeed({}, { fetchImpl, baseUrl: "https://api.example" });
    expect(seen[0]).toBe("https://api.example/api/telegram/v1/feed");
  });

  test("AbortSignal is propagated to fetch", async () => {
    let seenSignal: AbortSignal | undefined;
    const fetchImpl: typeof fetch = async (_input, init) => {
      seenSignal = init?.signal ?? undefined;
      return jsonResponse(200, SAMPLE);
    };
    const ctrl = new AbortController();
    await loadTelegramFeed({}, { fetchImpl, signal: ctrl.signal });
    expect(seenSignal).toBe(ctrl.signal);
  });
});

describe("parseIsoDatetime", () => {
  test("parses an ISO-8601 string", () => {
    expect(parseIsoDatetime("2026-05-04T12:00:00Z")).toBe(
      Date.UTC(2026, 4, 4, 12, 0, 0),
    );
  });

  test("returns null on empty / invalid input", () => {
    expect(parseIsoDatetime("")).toBeNull();
    expect(parseIsoDatetime("not a date")).toBeNull();
  });
});
