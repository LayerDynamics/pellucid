import { afterEach, describe, expect, test } from "bun:test";

import {
  tauriRuntime,
  telegramLoginRequestCode,
  telegramLoginSubmitCode,
  telegramLoginSubmitPassword,
  telegramLogout,
  telegramSessionPresent,
} from "./telegramLogin";

interface TauriInternalsLike {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
}

function installFakeTauri(handler: TauriInternalsLike): void {
  (globalThis as unknown as { __TAURI_INTERNALS__: TauriInternalsLike })
    .__TAURI_INTERNALS__ = handler;
  (globalThis as unknown as { window: typeof globalThis }).window = globalThis;
}

function clearFakeTauri(): void {
  delete (globalThis as unknown as { __TAURI_INTERNALS__?: unknown })
    .__TAURI_INTERNALS__;
}

afterEach(() => {
  clearFakeTauri();
});

describe("telegramLogin loader", () => {
  test("returns unavailable when Tauri runtime is absent", async () => {
    const out = await telegramLoginRequestCode("+15555550000");
    expect(out.kind).toBe("unavailable");
  });

  test("invokes telegram_login_request_code with the phone arg", async () => {
    const calls: { cmd: string; args: Record<string, unknown> | undefined }[] =
      [];
    installFakeTauri({
      invoke: async (cmd, args) => {
        calls.push({ cmd, args });
        return { phone: "+15555550000" };
      },
    });
    const out = await telegramLoginRequestCode("+15555550000");
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") {
      expect(out.value.phone).toBe("+15555550000");
    }
    expect(calls).toEqual([
      { cmd: "telegram_login_request_code", args: { phone: "+15555550000" } },
    ]);
  });

  test("submit_code passes through the response payload", async () => {
    installFakeTauri({
      invoke: async () => ({ ok: true, needs_password: false }),
    });
    const out = await telegramLoginSubmitCode("12345");
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") {
      expect(out.value.ok).toBe(true);
      expect(out.value.needs_password).toBe(false);
    }
  });

  test("submit_code with NeedsPassword flips the boolean", async () => {
    installFakeTauri({
      invoke: async () => ({ ok: false, needs_password: true }),
    });
    const out = await telegramLoginSubmitCode("12345");
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") {
      expect(out.value.needs_password).toBe(true);
    }
  });

  test("submit_password forwards the password arg", async () => {
    let received: unknown;
    installFakeTauri({
      invoke: async (_cmd, args) => {
        received = args;
        return { ok: true, needs_password: false };
      },
    });
    const out = await telegramLoginSubmitPassword("hunter2");
    expect(out.kind).toBe("ready");
    expect(received).toEqual({ password: "hunter2" });
  });

  test("logout invokes the right command", async () => {
    let cmdSeen = "";
    installFakeTauri({
      invoke: async (cmd) => {
        cmdSeen = cmd;
        return undefined;
      },
    });
    const out = await telegramLogout();
    expect(out.kind).toBe("ready");
    expect(cmdSeen).toBe("telegram_logout");
  });

  test("session_present returns the boolean", async () => {
    installFakeTauri({ invoke: async () => true });
    const out = await telegramSessionPresent();
    expect(out.kind).toBe("ready");
    if (out.kind === "ready") {
      expect(out.value).toBe(true);
    }
  });

  test("structured LoginError comes back via the error branch", async () => {
    installFakeTauri({
      invoke: async () => {
        throw { code: "code_invalid", message: "rejected" };
      },
    });
    const out = await telegramLoginSubmitCode("999");
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.error.code).toBe("code_invalid");
      expect(out.error.message).toBe("rejected");
    }
  });

  test("non-LoginError throws are normalised to mtproto", async () => {
    installFakeTauri({
      invoke: async () => {
        throw "unexpected string";
      },
    });
    const out = await telegramLoginRequestCode("+1");
    expect(out.kind).toBe("error");
    if (out.kind === "error") {
      expect(out.error.code).toBe("mtproto");
      expect(out.error.message).toBe("unexpected string");
    }
  });

  test("tauriRuntime returns undefined in non-Tauri context", () => {
    expect(tauriRuntime()).toBeUndefined();
  });
});
