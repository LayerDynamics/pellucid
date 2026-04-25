import { v } from "convex/values";

import { mutation, query } from "./_generated/server";

/**
 * Register an email on the pre-launch waitlist. Idempotent: returning the
 * existing row if the email is already present.
 */
export const register = mutation({
  args: {
    email: v.string(),
    source: v.string(),
    user_agent: v.optional(v.string()),
    referrer: v.optional(v.string()),
  },
  handler: async (ctx, args) => {
    const email = args.email.trim().toLowerCase();
    const source = args.source.trim();

    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
      throw new Error("invalid email");
    }
    if (source.length === 0 || source.length > 100) {
      throw new Error("source must be 1..100 chars");
    }

    const existing = await ctx.db
      .query("waitlist")
      .withIndex("by_email", (q) => q.eq("email", email))
      .first();
    if (existing) {
      return { id: existing._id, deduplicated: true };
    }

    const id = await ctx.db.insert("waitlist", {
      email,
      source,
      registered_at_ms: Date.now(),
      referrer: args.referrer,
      user_agent: args.user_agent,
    });

    return { id, deduplicated: false };
  },
});

/**
 * Total registered waitlist count — used by the marketing site banner
 * and for ops dashboards.
 */
export const count = query({
  args: {},
  handler: async (ctx) => {
    const docs = await ctx.db.query("waitlist").collect();
    return docs.length;
  },
});
