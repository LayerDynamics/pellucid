import { type FullConfig } from "@playwright/test";

/**
 * Playwright globalSetup hook (runs once before any test).
 *
 * At T0.8 the only job is to log the test target so CI runs are
 * self-documenting. Subsequent tasks layer in:
 *   - T2.6 — boot pellucid-edge-bin in test mode for the web e2e suite
 *   - T1.9 — boot pellucid-sidecar-bin against the desktop tauri build
 */
export default async function globalSetup(config: FullConfig): Promise<void> {
  const target = process.env.PELLUCID_E2E_BASE_URL ?? "http://127.0.0.1:5173";
  // eslint-disable-next-line no-console
  console.log(
    `[pellucid e2e] target=${target} projects=${config.projects.map((p) => p.name).join(",")}`,
  );
}
