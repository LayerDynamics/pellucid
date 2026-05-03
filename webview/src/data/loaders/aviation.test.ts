import { afterEach, describe, expect, test } from "bun:test";

import {
  loadFlightStatus,
  type FlightStatusOutcome,
} from "./aviation";

afterEach(() => {
  /* no shared state */
});

function fixedFetch(
  status: number,
  body: unknown,
  headers: Record<string, string> = {},
): typeof fetch {
  return (async () =>
    new Response(typeof body === "string" ? body : JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json", ...headers },
    })) as typeof fetch;
}

interface SpyCall {
  url: string;
  init?: RequestInit;
}

function spyFetch(): {
  fetch: typeof fetch;
  calls: SpyCall[];
} {
  const calls: SpyCall[] = [];
  const f = (async (url: unknown, init?: RequestInit) => {
    const call: SpyCall = { url: String(url) };
    if (init) call.init = init;
    calls.push(call);
    return new Response(
      JSON.stringify({
        flight: "AA100",
        scheduled_departure: "2026-04-25T12:00:00Z",
        scheduled_arrival: "2026-04-25T15:00:00Z",
        status: "active",
        origin: "JFK",
        destination: "LAX",
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  }) as typeof fetch;
  return { fetch: f, calls };
}

describe("loadFlightStatus", () => {
  test("200 → ready outcome with parsed FlightStatus", async () => {
    const out = await loadFlightStatus(
      { flight: "AA100", date: "2026-04-25", origin: "JFK" },
      {
        fetchImpl: fixedFetch(200, {
          flight: "AA100",
          scheduled_departure: "2026-04-25T12:00:00Z",
          scheduled_arrival: "2026-04-25T15:00:00Z",
          status: "active",
          origin: "JFK",
          destination: "LAX",
          departure_gate: "A12",
        }),
      },
    );
    expect(out.kind).toBe("ready");
    if (out.kind !== "ready") throw new Error("type narrow");
    expect(out.status.flight).toBe("AA100");
    expect(out.status.status).toBe("active");
    expect(out.status.departure_gate).toBe("A12");
  });

  test("400 invalid_request → error outcome with code", async () => {
    const out = await loadFlightStatus(
      { flight: "", date: "2026-04-25", origin: "JFK" },
      {
        fetchImpl: fixedFetch(400, {
          error: { code: "invalid_request", message: "bad flight" },
        }),
      },
    );
    expect(out.kind).toBe("error");
    if (out.kind !== "error") throw new Error("type narrow");
    expect(out.code).toBe("invalid_request");
    expect(out.message).toBe("bad flight");
    expect(out.httpStatus).toBe(400);
  });

  test("502 upstream_failure → error outcome with handler-side code preserved", async () => {
    const out = await loadFlightStatus(
      { flight: "ZZ999", date: "2026-04-25", origin: "JFK" },
      {
        fetchImpl: fixedFetch(502, {
          error: { code: "upstream_failure", message: "flight not found" },
        }),
      },
    );
    expect(out.kind).toBe("error");
    if (out.kind !== "error") throw new Error("type narrow");
    expect(out.code).toBe("upstream_failure");
    expect(out.httpStatus).toBe(502);
  });

  test("403 entitlement_forbidden → tier-locked branch", async () => {
    const out = await loadFlightStatus(
      { flight: "AA100", date: "2026-04-25", origin: "JFK" },
      {
        fetchImpl: fixedFetch(403, {
          error: { code: "entitlement_forbidden", message: "upgrade" },
        }),
      },
    );
    expect(out.kind).toBe("error");
    if (out.kind !== "error") throw new Error("type narrow");
    expect(out.code).toBe("entitlement_forbidden");
  });

  test("503 + Retry-After captures retryAfterSecs", async () => {
    const out: FlightStatusOutcome = await loadFlightStatus(
      { flight: "AA100", date: "2026-04-25", origin: "JFK" },
      {
        fetchImpl: fixedFetch(
          503,
          { error: { code: "upstream_failure", message: "outage" } },
          { "retry-after": "30" },
        ),
      },
    );
    expect(out.kind).toBe("error");
    if (out.kind !== "error") throw new Error("type narrow");
    expect(out.retryAfterSecs).toBe(30);
  });

  test("network failure → error outcome with code=network", async () => {
    const out = await loadFlightStatus(
      { flight: "AA100", date: "2026-04-25", origin: "JFK" },
      {
        fetchImpl: (async () => {
          throw new Error("connect refused");
        }) as typeof fetch,
      },
    );
    expect(out.kind).toBe("error");
    if (out.kind !== "error") throw new Error("type narrow");
    expect(out.code).toBe("network");
    expect(out.message).toBe("connect refused");
    expect(out.httpStatus).toBe(0);
  });

  test("unknown error code falls back to 'unknown'", async () => {
    const out = await loadFlightStatus(
      { flight: "AA100", date: "2026-04-25", origin: "JFK" },
      {
        fetchImpl: fixedFetch(500, {
          error: { code: "totally_made_up", message: "x" },
        }),
      },
    );
    expect(out.kind).toBe("error");
    if (out.kind !== "error") throw new Error("type narrow");
    expect(out.code).toBe("unknown");
  });

  test("malformed JSON on 200 → error outcome", async () => {
    const out = await loadFlightStatus(
      { flight: "AA100", date: "2026-04-25", origin: "JFK" },
      { fetchImpl: fixedFetch(200, "not json {{{") },
    );
    expect(out.kind).toBe("error");
    if (out.kind !== "error") throw new Error("type narrow");
    expect(out.code).toBe("unknown");
    expect(out.message).toMatch(/parse:/);
  });

  test("URL is built from the query params", async () => {
    const { fetch, calls } = spyFetch();
    await loadFlightStatus(
      { flight: "AA100", date: "2026-04-25", origin: "JFK" },
      { baseUrl: "http://api.test", fetchImpl: fetch },
    );
    expect(calls.length).toBe(1);
    expect(calls[0]!.url).toBe(
      "http://api.test/api/aviation/v1/get-flight-status?flight=AA100&date=2026-04-25&origin=JFK",
    );
  });

  test("query params are URL-encoded", async () => {
    const { fetch, calls } = spyFetch();
    await loadFlightStatus(
      { flight: "AA 100", date: "2026/04/25", origin: "JFK" },
      { fetchImpl: fetch },
    );
    expect(calls[0]!.url).toContain("flight=AA%20100");
    expect(calls[0]!.url).toContain("date=2026%2F04%2F25");
  });

  test("bearer token forwarded as Authorization header", async () => {
    const { fetch, calls } = spyFetch();
    await loadFlightStatus(
      { flight: "AA100", date: "2026-04-25", origin: "JFK" },
      { fetchImpl: fetch, bearerToken: "tok-123" },
    );
    const headers = calls[0]!.init?.headers as Record<string, string>;
    expect(headers.authorization).toBe("Bearer tok-123");
  });

  test("no bearer token → no Authorization header", async () => {
    const { fetch, calls } = spyFetch();
    await loadFlightStatus(
      { flight: "AA100", date: "2026-04-25", origin: "JFK" },
      { fetchImpl: fetch },
    );
    const headers = calls[0]!.init?.headers as Record<string, string>;
    expect(headers.authorization).toBeUndefined();
  });
});
