import {
  useCallback,
  useEffect,
  useState,
  type FormEvent,
  type ReactElement,
} from "react";

import {
  Dialog,
  DialogContent,
} from "../../components/primitives/Dialog";
import {
  telegramLoginRequestCode,
  telegramLoginSubmitCode,
  telegramLoginSubmitPassword,
  type TelegramLoginErrorCode,
  type TelegramLoginOutcome,
  type TelegramRequestCodeResponse,
  type TelegramSubmitCodeResponse,
} from "../../data/loaders/intel/telegramLogin";

/** Telegram onboarding dialog — three-step state machine driven by the
 *  Tauri IPC commands the host adds in T4.5.0. */
export interface TelegramOnboardingDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Fired once the session lands in the vault and the sidecar has been
   *  notified. Parent panels typically refetch their data here. */
  onComplete?: () => void;
  /** Loader injection point — defaults to the real IPC wrappers. Tests
   *  pass mocks. */
  loaders?: TelegramLoginLoaders;
}

/** Pluggable IPC wrappers for testability. */
export interface TelegramLoginLoaders {
  requestCode?: (
    phone: string,
  ) => Promise<TelegramLoginOutcome<TelegramRequestCodeResponse>>;
  submitCode?: (
    code: string,
  ) => Promise<TelegramLoginOutcome<TelegramSubmitCodeResponse>>;
  submitPassword?: (
    password: string,
  ) => Promise<TelegramLoginOutcome<TelegramSubmitCodeResponse>>;
}

type Step = "phone" | "code" | "password" | "done";

interface DialogError {
  code: DialogErrorCode;
  message: string;
}

interface State {
  step: Step;
  phone: string;
  code: string;
  password: string;
  pending: boolean;
  error: DialogError | null;
}

const INITIAL_STATE: State = {
  step: "phone",
  phone: "",
  code: "",
  password: "",
  pending: false,
  error: null,
};

/** Synthetic error code the dialog uses when the runtime is not Tauri.
 *  The Rust IPC layer never emits this — it's webview-internal. */
const UNAVAILABLE_CODE = "unavailable" as const;
type DialogErrorCode = TelegramLoginErrorCode | typeof UNAVAILABLE_CODE;

/** User-facing copy for each error code. Branching on `code` (not the
 *  raw message) is the contract. */
function explainError(code: DialogErrorCode): string {
  switch (code) {
    case "api_credentials_missing":
      return "Telegram API credentials missing — set telegram_api_id and telegram_api_hash before signing in.";
    case "no_login_in_flight":
      return "Login flow lost; please start over.";
    case "phone_invalid":
      return "Telegram rejected that phone number — double-check the country code.";
    case "code_invalid":
      return "That SMS code didn't work — try again.";
    case "password_invalid":
      return "Password rejected. The login flow has been reset; please start again from your phone number.";
    case "vault":
      return "Local keychain refused to store the session.";
    case "session_store":
      return "Couldn't save the Telegram session locally.";
    case "unavailable":
      return "Telegram sign-in is only available in the desktop app.";
    case "mtproto":
    default:
      return "Couldn't reach Telegram. Try again in a moment.";
  }
}

export function TelegramOnboardingDialog(
  props: TelegramOnboardingDialogProps,
): ReactElement {
  const { open, onOpenChange, onComplete, loaders } = props;
  const [state, setState] = useState<State>(INITIAL_STATE);

  // Reset to a clean state every time the dialog opens.
  useEffect(() => {
    if (open) setState(INITIAL_STATE);
  }, [open]);

  const requestCode = loaders?.requestCode ?? telegramLoginRequestCode;
  const submitCode = loaders?.submitCode ?? telegramLoginSubmitCode;
  const submitPassword = loaders?.submitPassword ?? telegramLoginSubmitPassword;

  const apply = useCallback(
    (outcome: TelegramLoginOutcome<unknown>): "ok" | "error" | "unavailable" => {
      if (outcome.kind === "ready") return "ok";
      if (outcome.kind === "unavailable") {
        setState((s) => ({
          ...s,
          pending: false,
          error: {
            code: UNAVAILABLE_CODE,
            message: "",
          },
        }));
        return "unavailable";
      }
      setState((s) => ({
        ...s,
        pending: false,
        error: { code: outcome.error.code, message: outcome.error.message },
      }));
      return "error";
    },
    [],
  );

  const handlePhone = async (e: FormEvent<HTMLFormElement>): Promise<void> => {
    e.preventDefault();
    if (!state.phone.trim() || state.pending) return;
    setState((s) => ({ ...s, pending: true, error: null }));
    const outcome = await requestCode(state.phone.trim());
    if (apply(outcome) === "ok") {
      setState((s) => ({
        ...s,
        pending: false,
        step: "code",
      }));
    }
  };

  const handleCode = async (e: FormEvent<HTMLFormElement>): Promise<void> => {
    e.preventDefault();
    if (!state.code.trim() || state.pending) return;
    setState((s) => ({ ...s, pending: true, error: null }));
    const outcome = await submitCode(state.code.trim());
    if (apply(outcome) === "ok" && outcome.kind === "ready") {
      if (outcome.value.ok && !outcome.value.needs_password) {
        setState((s) => ({ ...s, pending: false, step: "done" }));
        onComplete?.();
        onOpenChange(false);
      } else if (outcome.value.needs_password) {
        setState((s) => ({ ...s, pending: false, step: "password" }));
      }
    }
  };

  const handlePassword = async (e: FormEvent<HTMLFormElement>): Promise<void> => {
    e.preventDefault();
    if (!state.password || state.pending) return;
    setState((s) => ({ ...s, pending: true, error: null }));
    const outcome = await submitPassword(state.password);
    if (apply(outcome) === "ok" && outcome.kind === "ready") {
      if (outcome.value.ok) {
        setState((s) => ({ ...s, pending: false, step: "done" }));
        onComplete?.();
        onOpenChange(false);
      }
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        heading="Sign in to Telegram"
        blurb="Pellucid uses your Telegram account to read public channels you follow."
      >
        {state.step === "phone" && (
          <form
            onSubmit={handlePhone}
            data-pellucid="telegram-onboarding-step-phone"
          >
            <label className="block text-sm">
              <span className="block text-[var(--pellucid-fg-muted)]">
                Phone number (with country code)
              </span>
              <input
                type="tel"
                inputMode="tel"
                autoComplete="tel"
                placeholder="+15555550000"
                value={state.phone}
                disabled={state.pending}
                onChange={(e) =>
                  setState((s) => ({ ...s, phone: e.target.value }))
                }
                className="mt-1 w-full rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] px-3 py-2 text-sm"
              />
            </label>
            <button
              type="submit"
              disabled={!state.phone.trim() || state.pending}
              className="mt-4 inline-flex w-full justify-center rounded-md bg-[var(--pellucid-accent)] px-3 py-2 text-sm font-medium text-[var(--pellucid-on-accent)] disabled:opacity-50"
            >
              {state.pending ? "Sending..." : "Send code"}
            </button>
          </form>
        )}

        {state.step === "code" && (
          <form
            onSubmit={handleCode}
            data-pellucid="telegram-onboarding-step-code"
          >
            <p className="text-sm text-[var(--pellucid-fg-muted)]">
              Enter the code Telegram just sent to{" "}
              <span className="font-mono">{state.phone}</span>.
            </p>
            <label className="mt-3 block text-sm">
              <span className="block text-[var(--pellucid-fg-muted)]">
                SMS code
              </span>
              <input
                type="text"
                inputMode="numeric"
                pattern="[0-9]*"
                maxLength={8}
                autoComplete="one-time-code"
                value={state.code}
                disabled={state.pending}
                onChange={(e) =>
                  setState((s) => ({ ...s, code: e.target.value }))
                }
                className="mt-1 w-full rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] px-3 py-2 text-sm"
              />
            </label>
            <button
              type="submit"
              disabled={!state.code.trim() || state.pending}
              className="mt-4 inline-flex w-full justify-center rounded-md bg-[var(--pellucid-accent)] px-3 py-2 text-sm font-medium text-[var(--pellucid-on-accent)] disabled:opacity-50"
            >
              {state.pending ? "Verifying..." : "Verify code"}
            </button>
          </form>
        )}

        {state.step === "password" && (
          <form
            onSubmit={handlePassword}
            data-pellucid="telegram-onboarding-step-password"
          >
            <p className="text-sm text-[var(--pellucid-fg-muted)]">
              Two-factor authentication is enabled on this account. Enter your
              cloud password.
            </p>
            <label className="mt-3 block text-sm">
              <span className="block text-[var(--pellucid-fg-muted)]">
                Cloud password
              </span>
              <input
                type="password"
                autoComplete="current-password"
                value={state.password}
                disabled={state.pending}
                onChange={(e) =>
                  setState((s) => ({ ...s, password: e.target.value }))
                }
                className="mt-1 w-full rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] px-3 py-2 text-sm"
              />
            </label>
            <button
              type="submit"
              disabled={!state.password || state.pending}
              className="mt-4 inline-flex w-full justify-center rounded-md bg-[var(--pellucid-accent)] px-3 py-2 text-sm font-medium text-[var(--pellucid-on-accent)] disabled:opacity-50"
            >
              {state.pending ? "Signing in..." : "Sign in"}
            </button>
          </form>
        )}

        {state.error ? (
          <p
            data-pellucid="telegram-onboarding-error"
            className="mt-3 rounded-md border border-[var(--pellucid-danger)] bg-[var(--pellucid-danger-soft)] px-3 py-2 text-sm text-[var(--pellucid-danger)]"
          >
            {explainError(state.error.code)}
          </p>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
