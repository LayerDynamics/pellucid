import { v } from "convex/values";

import { internalMutation, internalQuery } from "./_generated/server";

/**
 * Webhook idempotency table mutations — invoked by Dodo's HTTP action to
 * record a webhook_id once the signature has been verified. Re-delivery
 * with the same id is a no-op so handlers can return 200 immediately.
 *
 * Wired to the `webhook_seen` table; preserves OP-18 from SPEC-001 §2.
 */
export const recordIfNew = internalMutation({
  args: {
    webhook_id: v.string(),
    payload_hash: v.string(),
    source: v.string(),
  },
  handler: async (ctx, args) => {
    const existing = await ctx.db
      .query("webhook_seen")
      .withIndex("by_webhook_id", (q) => q.eq("webhook_id", args.webhook_id))
      .first();

    if (existing) {
      return { id: existing._id, isNew: false };
    }

    const id = await ctx.db.insert("webhook_seen", {
      webhook_id: args.webhook_id,
      received_at_ms: Date.now(),
      payload_hash: args.payload_hash,
      source: args.source,
    });

    return { id, isNew: true };
  },
});

export const wasSeen = internalQuery({
  args: { webhook_id: v.string() },
  handler: async (ctx, args) => {
    const doc = await ctx.db
      .query("webhook_seen")
      .withIndex("by_webhook_id", (q) => q.eq("webhook_id", args.webhook_id))
      .first();
    return doc !== null;
  },
});
