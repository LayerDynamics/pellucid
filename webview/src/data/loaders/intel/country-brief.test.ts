import { afterEach, describe, expect, test } from "bun:test";

import {
  loadCountryBrief,
  type CountryBriefOutcome,
  type CountryBriefResponse,
} from "./country-brief";

const SAMPLE: CountryBriefResponse = {
  country: "Iran",
  region: "Middle East",
  summary: "8 ACLED events, 1 GDELT incident, 0 Telegram mentions.",
  topActor: { name: "IRGC", events: 8 },
  topArticle: {
    url: "https://a/1",
    title: "T",
    domain: "a.com",
    seenDate: "20260504T120000Z",
  },
  totals: { events: 8, incidents: 1, messages: 0 },
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

describe("loadCountryBrief", () => {
  test("missing country → invalid_request without a network call", async () => {
    let called = false;
    const fetchImpl: typeof fetch = async () => {
      called = true;
      return jsonResponse(200, SAMPLE);
    };
    const out = await loadCountryBrief({ country: "" }, { fetchImpl });
    expect(called).toBe(false);
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("invalid_request");
  });

  test("whitespace-only country → invalid_request", async () => {
    const out = await loadCountryBrief(
      { country: "  " },
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
    const out = await loadCountryBrief({ country: "Iran" }, { fetchImpl });
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") expect(out.response).toEqual(SAMPLE);
    expect(calls[0]).toBe(
      "/api/intelligence/v1/country-brief?country=Iran",
    );
  });

  test("trims whitespace before encoding", async () => {
    const seen: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      seen.push(typeof input === "string" ? input : (input as URL).toString());
      return jsonResponse(200, SAMPLE);
    };
    await loadCountryBrief({ country: "  Iran  " }, { fetchImpl });
    expect(seen[0] ?? "").toContain("country=Iran");
  });

  test("network error → kind=error code=network", async () => {
    const fetchImpl: typeof fetch = async () => {
      throw new Error("boom");
    };
    const out = await loadCountryBrief({ country: "Iran" }, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("network");
  });

  test("503 with bootstrap_upstream_empty body + retry-after header", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(
        503,
        { error: { code: "bootstrap_upstream_empty", message: "x" } },
        { "retry-after": "30" },
      );
    const out: CountryBriefOutcome = await loadCountryBrief(
      { country: "Iran" },
      { fetchImpl },
    );
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.code).toBe("bootstrap_upstream_empty");
      expect(out.retryAfterSecs).toBe(30);
    }
  });

  test("400 invalid_request from server passes through", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(400, {
        error: { code: "invalid_request", message: "missing country" },
      });
    const out = await loadCountryBrief({ country: "Iran" }, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("invalid_request");
  });

  test("502 cache_failure passes through", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(502, {
        error: { code: "cache_failure", message: "db locked" },
      });
    const out = await loadCountryBrief({ country: "Iran" }, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("cache_failure");
  });

  test("unknown error code falls back to 'unknown'", async () => {
    const fetchImpl: typeof fetch = async () =>
      jsonResponse(500, { error: { code: "no_such", message: "x" } });
    const out = await loadCountryBrief({ country: "Iran" }, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.code).toBe("unknown");
  });

  test("unparseable error body still synthesises an envelope", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json", { status: 500 });
    const out = await loadCountryBrief({ country: "Iran" }, { fetchImpl });
    expect(out.kind).toBe("error");
    if (out.kind === "error") expect(out.message).toBe("HTTP 500");
  });

  test("200 with malformed body → kind=error code=unknown", async () => {
    const fetchImpl: typeof fetch = async () =>
      new Response("not json", {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    const out = await loadCountryBrief({ country: "Iran" }, { fetchImpl });
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
    await loadCountryBrief(
      { country: "Iran" },
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
    await loadCountryBrief(
      { country: "Iran" },
      { fetchImpl, baseUrl: "https://api.example" },
    );
    expect(seen[0]).toBe(
      "https://api.example/api/intelligence/v1/country-brief?country=Iran",
    );
  });

  test("AbortSignal is propagated to fetch", async () => {
    let seenSignal: AbortSignal | undefined;
    const fetchImpl: typeof fetch = async (_input, init) => {
      seenSignal = init?.signal ?? undefined;
      return jsonResponse(200, SAMPLE);
    };
    const ctrl = new AbortController();
    await loadCountryBrief(
      { country: "Iran" },
      { fetchImpl, signal: ctrl.signal },
    );
    expect(seenSignal).toBe(ctrl.signal);
  });
});
