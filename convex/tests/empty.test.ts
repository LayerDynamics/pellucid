/**
 * convex-test integration smoke — proves an in-memory deployment boots
 * against the schema. No real Convex cloud deployment is required.
 *
 * Runs under vitest because convex-test internally uses Vite's
 * `import.meta.glob` to auto-discover convex functions; bun:test does
 * not implement that API. The rest of the project remains on bun:test.
 */

import { convexTest } from "convex-test";
import { describe, expect, test } from "vitest";

import schema from "../schema";

const modules = import.meta.glob("../**/!(*.*.*)*s");

describe("convex deployment (integration)", () => {
  test("convexTest boots with the Pellucid schema and zero data", async () => {
    const t = convexTest(schema, modules);
    expect(t).toBeDefined();
    expect(typeof t.run).toBe("function");
  });

  test("schema accepts a valid contact_submissions insert and round-trips", async () => {
    const t = convexTest(schema, modules);
    const result = await t.run(async (ctx) => {
      const id = await ctx.db.insert("contact_submissions", {
        name: "Ada Lovelace",
        email: "ada@example.test",
        message: "Hello from the integration test.",
        submitted_at_ms: Date.now(),
      });
      const round = await ctx.db.get(id);
      return { id, round };
    });
    expect(result.round).toBeDefined();
    expect(result.round?.email).toBe("ada@example.test");
  });

  test("waitlist by_email index returns the right row", async () => {
    const t = convexTest(schema, modules);
    const found = await t.run(async (ctx) => {
      await ctx.db.insert("waitlist", {
        email: "ada@example.test",
        source: "marketing/launch",
        registered_at_ms: 1_000,
      });
      await ctx.db.insert("waitlist", {
        email: "grace@example.test",
        source: "marketing/launch",
        registered_at_ms: 2_000,
      });
      return await ctx.db
        .query("waitlist")
        .withIndex("by_email", (q) => q.eq("email", "grace@example.test"))
        .first();
    });
    expect(found?.email).toBe("grace@example.test");
    expect(found?.registered_at_ms).toBe(2_000);
  });

  test("entitlements stores nested features object verbatim", async () => {
    const t = convexTest(schema, modules);
    const stored = await t.run(async (ctx) => {
      const id = await ctx.db.insert("entitlements", {
        user_id: "user_test_123",
        tier: 1,
        features: {
          tier: 1,
          maxDashboards: 5,
          apiAccess: false,
          apiRateLimit: 600,
          prioritySupport: false,
          exportFormats: ["json"],
        },
        valid_until_ms: 9_999_999_999_999,
        plan_id: "pro_monthly",
        updated_at_ms: 1_000,
      });
      return await ctx.db.get(id);
    });
    expect(stored?.tier).toBe(1);
    expect(stored?.features.maxDashboards).toBe(5);
    expect(stored?.features.exportFormats).toEqual(["json"]);
  });

  test("webhook_seen by_webhook_id index acts as primary key", async () => {
    const t = convexTest(schema, modules);
    const result = await t.run(async (ctx) => {
      await ctx.db.insert("webhook_seen", {
        webhook_id: "evt_abc",
        received_at_ms: 1_000,
        payload_hash: "sha256:abc",
        source: "dodo",
      });
      const found = await ctx.db
        .query("webhook_seen")
        .withIndex("by_webhook_id", (q) => q.eq("webhook_id", "evt_abc"))
        .first();
      const missing = await ctx.db
        .query("webhook_seen")
        .withIndex("by_webhook_id", (q) => q.eq("webhook_id", "evt_missing"))
        .first();
      return { found, missing };
    });
    expect(result.found?.payload_hash).toBe("sha256:abc");
    expect(result.missing).toBeNull();
  });
});
