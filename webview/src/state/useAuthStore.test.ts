import { afterEach, describe, expect, test } from "bun:test";

import { useAuthStore } from "./useAuthStore";

import type { Entitlements } from "./useAuthStore";

const FAKE_ENT: Entitlements = {
  tier: 1,
  maxDashboards: 5,
  apiAccess: false,
  apiRateLimit: 600,
  prioritySupport: false,
  exportFormats: ["json"],
  validUntilMs: 9_999_999_999_999,
};

afterEach(() => {
  useAuthStore.getState().signOut();
  useAuthStore.setState({ isLoading: false });
});

describe("useAuthStore", () => {
  test("initial state is signed out", () => {
    const s = useAuthStore.getState();
    expect(s.userId).toBeNull();
    expect(s.email).toBeNull();
    expect(s.entitlements).toBeNull();
    expect(s.isLoading).toBe(false);
  });

  test("signIn populates user + token + entitlements", () => {
    useAuthStore.getState().signIn({
      userId: "u1",
      email: "user@example.test",
      clerkSessionToken: "tok",
      entitlements: { ...FAKE_ENT },
    });
    const s = useAuthStore.getState();
    expect(s.userId).toBe("u1");
    expect(s.email).toBe("user@example.test");
    expect(s.clerkSessionToken).toBe("tok");
    expect(s.entitlements?.tier).toBe(1);
  });

  test("signOut clears every field", () => {
    useAuthStore.getState().signIn({
      userId: "u1",
      email: "x@y",
      clerkSessionToken: "tok",
      entitlements: { ...FAKE_ENT },
    });
    useAuthStore.getState().signOut();
    const s = useAuthStore.getState();
    expect(s.userId).toBeNull();
    expect(s.entitlements).toBeNull();
  });

  test("setEntitlements + setLoading update independently", () => {
    useAuthStore.getState().setEntitlements({ ...FAKE_ENT, tier: 2 });
    expect(useAuthStore.getState().entitlements?.tier).toBe(2);
    useAuthStore.getState().setLoading(true);
    expect(useAuthStore.getState().isLoading).toBe(true);
  });

  test("hasTier compares against entitlement tier", () => {
    useAuthStore.getState().signIn({
      userId: "u",
      email: "x",
      clerkSessionToken: "t",
      entitlements: { ...FAKE_ENT, tier: 2 },
    });
    expect(useAuthStore.getState().hasTier(0)).toBe(true);
    expect(useAuthStore.getState().hasTier(1)).toBe(true);
    expect(useAuthStore.getState().hasTier(2)).toBe(true);
    expect(useAuthStore.getState().hasTier(3)).toBe(false);
  });

  test("hasTier returns false when entitlements null", () => {
    expect(useAuthStore.getState().hasTier(1)).toBe(false);
  });
});
