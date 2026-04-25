import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";

/**
 * Auth + entitlements snapshot. Populated during the 8-phase boot's
 * P3 (Clerk auth subscribe) and refreshed when entitlement TTL elapses.
 *
 * The `entitlements.tier` numeric mirrors SPEC-001 §1.5:
 *   0 free, 1 pro_*, 2 api_starter / api_business, 3 enterprise.
 */
export interface Entitlements {
  tier: number;
  maxDashboards: number;
  apiAccess: boolean;
  apiRateLimit: number;
  prioritySupport: boolean;
  exportFormats: string[];
  validUntilMs: number;
}

export interface AuthState {
  userId: string | null;
  email: string | null;
  clerkSessionToken: string | null;
  entitlements: Entitlements | null;
  isLoading: boolean;
  signIn: (params: {
    userId: string;
    email: string;
    clerkSessionToken: string;
    entitlements: Entitlements | null;
  }) => void;
  signOut: () => void;
  setEntitlements: (entitlements: Entitlements | null) => void;
  setLoading: (isLoading: boolean) => void;
  hasTier: (minTier: number) => boolean;
}

export const useAuthStore = create<AuthState>()(
  subscribeWithSelector((set, get) => ({
    userId: null,
    email: null,
    clerkSessionToken: null,
    entitlements: null,
    isLoading: false,
    signIn: ({ userId, email, clerkSessionToken, entitlements }) => {
      set({ userId, email, clerkSessionToken, entitlements, isLoading: false });
    },
    signOut: () => {
      set({
        userId: null,
        email: null,
        clerkSessionToken: null,
        entitlements: null,
        isLoading: false,
      });
    },
    setEntitlements: (entitlements) => set({ entitlements }),
    setLoading: (isLoading) => set({ isLoading }),
    hasTier: (minTier) => {
      const tier = get().entitlements?.tier ?? 0;
      return tier >= minTier;
    },
  })),
);
