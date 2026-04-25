import { defineSchema, defineTable } from "convex/server";
import { v } from "convex/values";

/**
 * Pellucid Convex schema (SPEC-001 §15).
 *
 * Convex is retained for billing/webhook plumbing only — Clerk session
 * verification, Dodo webhook idempotency, identity HMAC, and the
 * server-of-record entitlements snapshot that the Rust edge falls back
 * to on cache miss. Everything else (cache, gateway, seeders, streams,
 * ML, correlation) lives in the Rust workspace.
 *
 * Tables:
 *   - entitlements         user_id → tier + features + valid_until
 *                          (mirrored into pellucid-cache.entitlements_cache
 *                          with a 15-minute TTL — see crates/pellucid-auth)
 *   - webhook_seen         Dodo webhook_id → received_at + payload_hash
 *                          (idempotency, OP-18)
 *   - contact_submissions  marketing-site contact form (preserved from
 *                          the original WorldMonitor convex/ schema)
 *   - waitlist             pre-launch interest registrations
 */
export default defineSchema({
  entitlements: defineTable({
    user_id: v.string(),
    tier: v.number(),
    features: v.object({
      tier: v.number(),
      maxDashboards: v.number(),
      apiAccess: v.boolean(),
      apiRateLimit: v.number(),
      prioritySupport: v.boolean(),
      exportFormats: v.array(v.string()),
    }),
    valid_until_ms: v.number(),
    plan_id: v.string(),
    updated_at_ms: v.number(),
  })
    .index("by_user", ["user_id"])
    .index("by_validity", ["valid_until_ms"]),

  webhook_seen: defineTable({
    webhook_id: v.string(),
    received_at_ms: v.number(),
    payload_hash: v.string(),
    source: v.string(),
  })
    .index("by_webhook_id", ["webhook_id"])
    .index("by_received_at", ["received_at_ms"]),

  contact_submissions: defineTable({
    name: v.string(),
    email: v.string(),
    message: v.string(),
    submitted_at_ms: v.number(),
    user_agent: v.optional(v.string()),
    referrer: v.optional(v.string()),
  })
    .index("by_email", ["email"])
    .index("by_submitted_at", ["submitted_at_ms"]),

  waitlist: defineTable({
    email: v.string(),
    source: v.string(),
    registered_at_ms: v.number(),
    referrer: v.optional(v.string()),
    user_agent: v.optional(v.string()),
  })
    .index("by_email", ["email"])
    .index("by_registered_at", ["registered_at_ms"]),
});
