/**
 * Desktop e2e — gated MTProto onboarding test for the Tauri host
 * (T4.5.0).
 *
 * Drives the real `__TAURI__.invoke(...)` bridge through the three
 * IPC commands the onboarding dialog exercises:
 *  - `telegram_login_request_code(phone)`
 *  - `telegram_login_submit_code(code)`
 *  - `telegram_login_submit_password(password)` (only when 2FA)
 *
 * Gated behind `TELEGRAM_E2E_DESKTOP=1` because it requires real
 * sandbox credentials in CI secrets:
 *  - `TELEGRAM_TEST_PHONE` — sandbox phone
 *  - `TELEGRAM_TEST_BOT_TOKEN` — Bot API token for the SMS-receiver
 *  - `TELEGRAM_TEST_2FA` (optional) — 2FA password
 *
 * The Tauri host's `LocalApiState` is expected to already have
 * `telegram_api_id` / `telegram_api_hash` populated in the keychain
 * before this test runs (CI seeds them in a `beforeAll`). When
 * `__TAURI__` is not present (host not bundled in this shard) the
 * test soft-skips like the existing `ipc.spec.ts` family.
 */

import { browser, expect } from "@wdio/globals";

const TELEGRAM_E2E_DESKTOP = process.env["TELEGRAM_E2E_DESKTOP"] === "1";

describe("@telegram desktop onboarding", () => {
  it("phone -> code -> (optional 2FA) flow leaves a session in the vault", async () => {
    if (!TELEGRAM_E2E_DESKTOP) {
      console.log(
        "skipped: TELEGRAM_E2E_DESKTOP=1 required (paired sandbox + bot in CI secrets)",
      );
      return;
    }
    const phone = process.env["TELEGRAM_TEST_PHONE"];
    if (!phone) {
      throw new Error("TELEGRAM_TEST_PHONE must be set");
    }

    await browser.url("/");
    const present = await browser.execute(async () => {
      const tauri = (
        window as unknown as {
          __TAURI__?: { invoke: (c: string) => Promise<unknown> };
        }
      ).__TAURI__;
      if (!tauri) return null;
      return tauri.invoke("telegram_session_present");
    });
    if (present === null) {
      console.log("skipped: __TAURI__ bridge not present in this shard");
      return;
    }

    const requestRes = await browser.execute(
      async (phoneArg: string) => {
        const tauri = (
          window as unknown as {
            __TAURI__?: {
              invoke: (
                cmd: string,
                args?: Record<string, string>,
              ) => Promise<{ phone: string }>;
            };
          }
        ).__TAURI__;
        if (!tauri) return null;
        return tauri.invoke("telegram_login_request_code", { phone: phoneArg });
      },
      phone,
    );
    expect(requestRes, "request_login_code returned null").not.toBeNull();
    expect(requestRes!.phone).toEqual(phone);

    const code = await fetchSmsCodeFromBot();
    expect(code).toMatch(/^\d{4,8}$/);

    const codeRes = await browser.execute(
      async (codeArg: string) => {
        const tauri = (
          window as unknown as {
            __TAURI__?: {
              invoke: (
                cmd: string,
                args?: Record<string, string>,
              ) => Promise<{ ok: boolean; needs_password: boolean }>;
            };
          }
        ).__TAURI__;
        if (!tauri) return null;
        return tauri.invoke("telegram_login_submit_code", { code: codeArg });
      },
      code,
    );
    expect(codeRes).not.toBeNull();

    if (!codeRes!.ok && codeRes!.needs_password) {
      const password = process.env["TELEGRAM_TEST_2FA"];
      if (!password) {
        throw new Error(
          "TELEGRAM_TEST_2FA env var required when sandbox account has 2FA",
        );
      }
      const pwRes = await browser.execute(
        async (passArg: string) => {
          const tauri = (
            window as unknown as {
              __TAURI__?: {
                invoke: (
                  cmd: string,
                  args?: Record<string, string>,
                ) => Promise<{ ok: boolean; needs_password: boolean }>;
              };
            }
          ).__TAURI__;
          if (!tauri) return null;
          return tauri.invoke("telegram_login_submit_password", {
            password: passArg,
          });
        },
        password,
      );
      expect(pwRes).not.toBeNull();
      expect(pwRes!.ok).toBe(true);
    } else {
      expect(codeRes!.ok).toBe(true);
    }

    // Confirm the session landed in the vault.
    const finalPresent = await browser.execute(async () => {
      const tauri = (
        window as unknown as {
          __TAURI__?: { invoke: (c: string) => Promise<boolean> };
        }
      ).__TAURI__;
      if (!tauri) return false;
      return tauri.invoke("telegram_session_present");
    });
    expect(finalPresent).toBe(true);
  });
});

async function fetchSmsCodeFromBot(): Promise<string> {
  const token = process.env["TELEGRAM_TEST_BOT_TOKEN"];
  if (!token) throw new Error("TELEGRAM_TEST_BOT_TOKEN not set");
  const deadline = Date.now() + 30_000;
  let lastUpdateId = 0;
  while (Date.now() < deadline) {
    const url = `https://api.telegram.org/bot${token}/getUpdates?offset=${
      lastUpdateId + 1
    }&timeout=5`;
    const res = await fetch(url);
    if (!res.ok) {
      await new Promise((r) => setTimeout(r, 1_000));
      continue;
    }
    const json = (await res.json()) as {
      ok: boolean;
      result: Array<{ update_id: number; message?: { text?: string } }>;
    };
    if (!json.ok || !Array.isArray(json.result)) continue;
    for (const u of json.result) {
      lastUpdateId = Math.max(lastUpdateId, u.update_id);
      const text = u.message?.text ?? "";
      const match = /\b(\d{4,8})\b/.exec(text);
      if (match) return match[1]!;
    }
  }
  throw new Error("timed out waiting for SMS code from test bot");
}
