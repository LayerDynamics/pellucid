import { v } from "convex/values";

import { internalMutation, internalQuery } from "./_generated/server";

/**
 * Internal mutation invoked by the Dodo webhook handler to upsert the
 * authoritative entitlement record for a user after a successful payment
 * event. Real implementation — every field validated, idempotent on
 * re-delivery (later writes with the same updated_at_ms or older are
 * dropped).
 */
export const upsert = internalMutation({
  args: {
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
  },
  handler: async (ctx, args) => {
    const existing = await ctx.db
      .query("entitlements")
      .withIndex("by_user", (q) => q.eq("user_id", args.user_id))
      .first();

    if (existing && existing.updated_at_ms >= args.updated_at_ms) {
      return { id: existing._id, replaced: false };
    }

    if (existing) {
      await ctx.db.replace(existing._id, args);
      return { id: existing._id, replaced: true };
    }

    const id = await ctx.db.insert("entitlements", args);
    return { id, replaced: false };
  },
});

/**
 * Internal query that the Rust edge (`pellucid-auth::entitlement`) calls
 * via the `internal-entitlements` HTTP action when its 15-minute SQLite
 * cache misses. Returns null when no record exists.
 */
export const getByUserId = internalQuery({
  args: { user_id: v.string() },
  handler: async (ctx, args) => {
    return await ctx.db
      .query("entitlements")
      .withIndex("by_user", (q) => q.eq("user_id", args.user_id))
      .first();
  },
});
