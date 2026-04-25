import { create } from "zustand";
import { persist, subscribeWithSelector } from "zustand/middleware";

/** Domain-skinned variants per SPEC-001 §16. */
export type Variant = "base" | "tech" | "finance" | "commodity" | "happy";

export const ALL_VARIANTS: readonly Variant[] = [
  "base",
  "tech",
  "finance",
  "commodity",
  "happy",
] as const;

export interface VariantState {
  variant: Variant;
  switching: boolean;
  available: readonly Variant[];
  setVariant: (variant: Variant) => void;
  setSwitching: (switching: boolean) => void;
}

const isValidVariant = (v: unknown): v is Variant =>
  typeof v === "string" && (ALL_VARIANTS as readonly string[]).includes(v);

export const useVariantStore = create<VariantState>()(
  persist(
    subscribeWithSelector((set) => ({
      variant: "base",
      switching: false,
      available: ALL_VARIANTS,
      setVariant: (variant) => {
        if (!isValidVariant(variant)) return;
        set({ variant, switching: true });
        // The reactions module clears `switching` once the cross-store
        // reset (map layers, panel allow-list) has fired.
      },
      setSwitching: (switching) => set({ switching }),
    })),
    {
      name: "pellucid:variant",
      partialize: (s) => ({ variant: s.variant }),
    },
  ),
);
