/** Shared error-code label helper used by every macro panel. */

export function labelForCode(code: string): string {
  switch (code) {
    case "invalid_request":
      return "Invalid request";
    case "cache_failure":
      return "Cache failure";
    case "cache_shape":
      return "Wire-shape mismatch";
    case "entitlement_forbidden":
      return "Premium tier required";
    case "clerk_unauthorized":
      return "Sign-in required";
    case "network":
      return "Network error";
    default:
      return "Error";
  }
}

/** Format wall-clock ms as `HH:MM UTC`. */
export function formatAssembledAtUtc(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const d = new Date(ms);
  const hh = String(d.getUTCHours()).padStart(2, "0");
  const mm = String(d.getUTCMinutes()).padStart(2, "0");
  return `${hh}:${mm} UTC`;
}
