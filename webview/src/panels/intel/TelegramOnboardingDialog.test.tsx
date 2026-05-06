import { afterEach, describe, expect, test } from "bun:test";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";

import { TelegramOnboardingDialog } from "./TelegramOnboardingDialog";
import type {
  TelegramLoginOutcome,
  TelegramRequestCodeResponse,
  TelegramSubmitCodeResponse,
} from "../../data/loaders/intel/telegramLogin";

afterEach(() => {
  cleanup();
});

function ready<T>(value: T): TelegramLoginOutcome<T> {
  return { kind: "ready", value };
}

function err<T>(
  code:
    | "code_invalid"
    | "phone_invalid"
    | "password_invalid"
    | "mtproto",
): TelegramLoginOutcome<T> {
  return {
    kind: "error",
    error: { code, message: "test failure" },
  };
}

function setInputValue(input: HTMLInputElement, value: string): void {
  fireEvent.change(input, { target: { value } });
}

function submitForm(input: HTMLInputElement): void {
  const form = input.closest("form");
  if (!form) throw new Error("input not inside a form");
  fireEvent.submit(form);
}

describe("TelegramOnboardingDialog", () => {
  test("phone -> code (Done) flow dismisses the dialog and fires onComplete", async () => {
    let openState = true;
    let onCompleteFired = 0;
    const requestCode = async (
      phone: string,
    ): Promise<TelegramLoginOutcome<TelegramRequestCodeResponse>> =>
      ready({ phone });
    const submitCode = async (
      _: string,
    ): Promise<TelegramLoginOutcome<TelegramSubmitCodeResponse>> =>
      ready({ ok: true, needs_password: false });

    render(
      <TelegramOnboardingDialog
        open={openState}
        onOpenChange={(o) => {
          openState = o;
        }}
        onComplete={() => {
          onCompleteFired += 1;
        }}
        loaders={{ requestCode, submitCode }}
      />,
    );

    const phoneInput = screen.getByPlaceholderText(
      "+15555550000",
    ) as HTMLInputElement;
    setInputValue(phoneInput, "+15555550000");
    submitForm(phoneInput);

    const codeInput = await waitFor(() =>
      screen.getByLabelText(/SMS code/i) as HTMLInputElement,
    );
    setInputValue(codeInput, "12345");
    const codeForm = codeInput.closest("form")!;
    codeForm.dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    await waitFor(() => {
      expect(openState).toBe(false);
      expect(onCompleteFired).toBe(1);
    });
  });

  test("phone -> code (NeedsPassword) -> password flow", async () => {
    let openState = true;
    let onCompleteFired = 0;
    let submittedPassword: string | undefined;
    const requestCode = async (phone: string) => ready({ phone });
    const submitCode = async (_: string) =>
      ready({ ok: false, needs_password: true });
    const submitPassword = async (password: string) => {
      submittedPassword = password;
      return ready({ ok: true, needs_password: false });
    };

    render(
      <TelegramOnboardingDialog
        open={openState}
        onOpenChange={(o) => {
          openState = o;
        }}
        onComplete={() => {
          onCompleteFired += 1;
        }}
        loaders={{ requestCode, submitCode, submitPassword }}
      />,
    );

    const phoneInput = screen.getByPlaceholderText(
      "+15555550000",
    ) as HTMLInputElement;
    setInputValue(phoneInput, "+15555550000");
    submitForm(phoneInput);

    const codeInput = await waitFor(() =>
      screen.getByLabelText(/SMS code/i) as HTMLInputElement,
    );
    setInputValue(codeInput, "11111");
    submitForm(codeInput);

    const passwordInput = await waitFor(() =>
      screen.getByLabelText(/Cloud password/i) as HTMLInputElement,
    );
    setInputValue(passwordInput, "hunter2");
    submitForm(passwordInput);

    await waitFor(() => {
      expect(openState).toBe(false);
      expect(submittedPassword).toBe("hunter2");
      expect(onCompleteFired).toBe(1);
    });
  });

  test("error from submitCode is rendered with the user-facing copy", async () => {
    const requestCode = async (phone: string) => ready({ phone });
    const submitCode = async (_: string) =>
      err<TelegramSubmitCodeResponse>("code_invalid");

    render(
      <TelegramOnboardingDialog
        open={true}
        onOpenChange={() => {}}
        loaders={{ requestCode, submitCode }}
      />,
    );

    const phoneInput = screen.getByPlaceholderText(
      "+15555550000",
    ) as HTMLInputElement;
    setInputValue(phoneInput, "+1");
    submitForm(phoneInput);

    const codeInput = await waitFor(() =>
      screen.getByLabelText(/SMS code/i) as HTMLInputElement,
    );
    setInputValue(codeInput, "999");
    submitForm(codeInput);

    await waitFor(() => {
      const banner = screen.getByText(/SMS code didn't work/i);
      expect(banner).toBeDefined();
    });
  });

  test("unavailable runtime renders the desktop-only banner", async () => {
    const requestCode = async (
      _phone: string,
    ): Promise<TelegramLoginOutcome<TelegramRequestCodeResponse>> => ({
      kind: "unavailable",
    });

    render(
      <TelegramOnboardingDialog
        open={true}
        onOpenChange={() => {}}
        loaders={{ requestCode }}
      />,
    );

    const phoneInput = screen.getByPlaceholderText(
      "+15555550000",
    ) as HTMLInputElement;
    setInputValue(phoneInput, "+1");
    submitForm(phoneInput);

    await waitFor(() => {
      const banner = screen.getByText(/desktop app/i);
      expect(banner).toBeDefined();
    });
  });
});
