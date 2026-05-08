import { defineConfig, devices } from "@playwright/test";

const isCI = !!process.env.CI;
const baseURL = process.env.PELLUCID_E2E_BASE_URL ?? "http://127.0.0.1:5173";
// `web-visual` project asserts pixel-precise screenshots against
// committed goldens. Goldens are platform-specific (darwin / linux),
// and the primitives showcase route (T1.11+) hasn't shipped yet so the
// per-variant tests time out waiting for `[data-pellucid-showcase="root"]`.
// Gate the visual project behind an explicit env opt-in so CI only
// runs it when goldens for the current platform are committed.
const runVisual = process.env.PELLUCID_RUN_VISUAL === "1";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: isCI,
  retries: isCI ? 2 : 0,
  workers: isCI ? 2 : undefined,
  reporter: isCI
    ? [["html", { open: "never" }], ["github"]]
    : [["list"], ["html", { open: "never" }]],
  timeout: 30_000,
  expect: {
    timeout: 5_000,
    toHaveScreenshot: {
      maxDiffPixelRatio: 0.005,
      threshold: 0.2,
    },
  },
  globalSetup: "./e2e/fixtures/global-setup.ts",
  use: {
    baseURL,
    trace: "on-first-retry",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
  },
  projects: [
    {
      name: "web",
      use: { ...devices["Desktop Chrome"] },
      testIgnore: ["**/desktop/**", "**/visual/**"],
    },
    ...(runVisual
      ? [
          {
            name: "web-visual",
            use: {
              ...devices["Desktop Chrome"],
              viewport: { width: 1440, height: 900 },
              deviceScaleFactor: 2,
            },
            testMatch: ["**/visual/**"],
          },
        ]
      : []),
  ],
  webServer: {
    command: "bun run --filter=@pellucid/webview dev",
    url: baseURL,
    reuseExistingServer: !isCI,
    timeout: 60_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
