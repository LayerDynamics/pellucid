/**
 * Schema-shape unit tests. These run on the raw schema definition without
 * a deployment, so they exercise the Convex schema validator path (which
 * runs at module import time) and assert every required table is present
 * per SPEC-001 §15.
 */

import { describe, expect, test } from "vitest";

import schema from "../schema";

describe("convex schema (unit)", () => {
  const tables = (schema as unknown as { tables: Record<string, unknown> }).tables;

  test("module exports a schema with the four required tables", () => {
    expect(typeof tables).toBe("object");
    for (const required of ["entitlements", "webhook_seen", "contact_submissions", "waitlist"]) {
      expect(tables).toHaveProperty(required);
    }
  });

  test("schema is a valid SchemaDefinition", () => {
    expect(schema).toBeDefined();
    expect(typeof (schema as unknown as { export: unknown }).export).toBe("function");
  });
});
