/**
 * Convex auth configuration — Clerk JWT verification.
 *
 * The Convex deployment trusts JWTs issued by Clerk for the configured
 * frontend application. The Rust edge (`pellucid-auth::clerk`) performs
 * the same verification independently against the same JWKS; this config
 * exists so Convex queries/mutations that need an authenticated user
 * (e.g. /api/internal-entitlements caller resolution) can use Convex's
 * built-in `ctx.auth.getUserIdentity()`.
 *
 * `CLERK_JWT_ISSUER_DOMAIN` is set in the Convex deployment via
 * `bunx convex env set CLERK_JWT_ISSUER_DOMAIN <https://...>` during
 * provisioning. Empty default is intentional — it forces a deploy-time
 * configuration error rather than silently accepting unsigned tokens.
 */
export default {
  providers: [
    {
      domain: process.env.CLERK_JWT_ISSUER_DOMAIN ?? "",
      applicationID: "convex",
    },
  ],
};
