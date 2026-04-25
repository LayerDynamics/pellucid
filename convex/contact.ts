import { v } from "convex/values";

import { mutation, query } from "./_generated/server";

/**
 * Submit a contact-form entry from the marketing site. Real implementation
 * — performs basic validation, persists, and returns the new document id.
 */
export const submit = mutation({
  args: {
    name: v.string(),
    email: v.string(),
    message: v.string(),
    user_agent: v.optional(v.string()),
    referrer: v.optional(v.string()),
  },
  handler: async (ctx, args) => {
    const name = args.name.trim();
    const email = args.email.trim().toLowerCase();
    const message = args.message.trim();

    if (name.length === 0 || name.length > 200) {
      throw new Error("name must be 1..200 chars");
    }
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
      throw new Error("invalid email");
    }
    if (message.length === 0 || message.length > 10_000) {
      throw new Error("message must be 1..10000 chars");
    }

    const id = await ctx.db.insert("contact_submissions", {
      name,
      email,
      message,
      submitted_at_ms: Date.now(),
      user_agent: args.user_agent,
      referrer: args.referrer,
    });

    return { id };
  },
});

/**
 * Look up the most recent contact submission for an email. Used by admin
 * triage tooling.
 */
export const latestForEmail = query({
  args: { email: v.string() },
  handler: async (ctx, args) => {
    const email = args.email.trim().toLowerCase();
    const docs = await ctx.db
      .query("contact_submissions")
      .withIndex("by_email", (q) => q.eq("email", email))
      .order("desc")
      .take(1);
    return docs[0] ?? null;
  },
});
