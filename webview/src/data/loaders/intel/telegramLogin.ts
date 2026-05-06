/**
 * Telegram MTProto onboarding loader — desktop-only.
 *
 * Wraps the five Tauri IPC commands the host adds in T4.5.0:
 * - `telegram_login_request_code(phone)` → `RequestCodeResponse`
 * - `telegram_login_submit_code(code)`   → `SubmitCodeResponse`
 * - `telegram_login_submit_password(password)` → `SubmitCodeResponse`
 * - `telegram_logout()` → `void`
 * - `telegram_session_present()` → `boolean`
 *
 * Wire shapes mirror the Rust types in
 * `crates/pellucid-tauri/src/telegram_login.rs` (`RequestCodeResponse`,
 * `SubmitCodeResponse`, `LoginError`). The error code strings are the
 * stable surface — UI strings must branch on `err.code`, never
 * `err.message`.
 *
 * In hosted web mode (no Tauri runtime) every call returns
 * `{ kind: "unavailable" }` so panels can degrade gracefully without
 * importing `@tauri-apps/api`.
 */

/** Wire shape from `RequestCodeResponse` in the Rust module. */
export interface TelegramRequestCodeResponse {
  /** Echo of the phone number the host accepted. */
  phone: string;
}

/** Wire shape from `SubmitCodeResponse` in the Rust module. */
export interface TelegramSubmitCodeResponse {
  /** `true` when the underlying session is now authenticated. */
  ok: boolean;
  /**
   * `true` when the account has 2FA — caller follows up with
   * `telegramLoginSubmitPassword`.
   */
  needs_password: boolean;
}

/** Stable error codes. Mirrors `LoginError::code` in the Rust module. */
export type TelegramLoginErrorCode =
  | "api_credentials_missing"
  | "no_login_in_flight"
  | "phone_invalid"
  | "code_invalid"
  | "password_invalid"
  | "mtproto"
  | "vault"
  | "session_store";

/** Shape of the `LoginError` JSON the host sends back via Tauri. */
export interface TelegramLoginError {
  code: TelegramLoginErrorCode;
  message: string;
}

/** Discriminated outcome the dialog branches on. */
export type TelegramLoginOutcome<T> =
  | { kind: "ready"; value: T }
  | { kind: "error"; error: TelegramLoginError }
  | { kind: "unavailable" };

interface TauriInternals {
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
}

/**
 * Detect whether the page is running inside the Tauri runtime. We
 * read `window.__TAURI_INTERNALS__` rather than depending on
 * `@tauri-apps/api` so the hosted web build doesn't pull a desktop-
 * only package.
 */
export function tauriRuntime(): TauriInternals | undefined {
  if (typeof window === "undefined") return undefined;
  const w = window as unknown as { __TAURI_INTERNALS__?: TauriInternals };
  return w.__TAURI_INTERNALS__;
}

async function invokeOrUnavailable<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<TelegramLoginOutcome<T>> {
  const tauri = tauriRuntime();
  if (!tauri) {
    return { kind: "unavailable" };
  }
  try {
    const value = await tauri.invoke<T>(cmd, args);
    return { kind: "ready", value };
  } catch (raw) {
    return { kind: "error", error: normaliseError(raw) };
  }
}

function normaliseError(raw: unknown): TelegramLoginError {
  if (
    raw &&
    typeof raw === "object" &&
    "code" in raw &&
    typeof (raw as { code: unknown }).code === "string"
  ) {
    const r = raw as { code: TelegramLoginErrorCode; message?: string };
    return {
      code: r.code,
      message: typeof r.message === "string" ? r.message : "",
    };
  }
  return {
    code: "mtproto",
    message: typeof raw === "string" ? raw : JSON.stringify(raw ?? null),
  };
}

/** Step 1: ask Telegram to send the SMS code. */
export function telegramLoginRequestCode(
  phone: string,
): Promise<TelegramLoginOutcome<TelegramRequestCodeResponse>> {
  return invokeOrUnavailable("telegram_login_request_code", { phone });
}

/** Step 2: submit the SMS code. */
export function telegramLoginSubmitCode(
  code: string,
): Promise<TelegramLoginOutcome<TelegramSubmitCodeResponse>> {
  return invokeOrUnavailable("telegram_login_submit_code", { code });
}

/** Step 3 (only when `needs_password`): submit the 2FA password. */
export function telegramLoginSubmitPassword(
  password: string,
): Promise<TelegramLoginOutcome<TelegramSubmitCodeResponse>> {
  return invokeOrUnavailable("telegram_login_submit_password", { password });
}

/** Clear the stored session; sidecar drains its run task. */
export function telegramLogout(): Promise<TelegramLoginOutcome<void>> {
  return invokeOrUnavailable("telegram_logout");
}

/** `true` when the host vault carries a non-empty `telegram_session`. */
export function telegramSessionPresent(): Promise<TelegramLoginOutcome<boolean>> {
  return invokeOrUnavailable("telegram_session_present");
}
