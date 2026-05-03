import type { Variant } from "../../state/useVariantStore";
import { baseConfig } from "./base";
import { commodityConfig } from "./commodity";
import { financeConfig } from "./finance";
import { happyConfig } from "./happy";
import { techConfig } from "./tech";
import type { VariantConfig } from "./types";

export type { VariantConfig } from "./types";

/** Lookup map: every variant id → its config. */
export const VARIANT_CONFIGS: Readonly<Record<Variant, VariantConfig>> = {
  base: baseConfig,
  tech: techConfig,
  finance: financeConfig,
  commodity: commodityConfig,
  happy: happyConfig,
};

/** Look up a variant config by id. Throws on unknown ids — the
 *  type system rules out anything but the 5 known variants, so
 *  the throw is unreachable at compile time but we surface a
 *  helpful runtime error in case the type system is ever
 *  bypassed (e.g. test fixtures, JSON drift). */
export function configFor(variant: Variant): VariantConfig {
  const cfg = VARIANT_CONFIGS[variant];
  if (!cfg) {
    throw new Error(`unknown variant: ${variant}`);
  }
  return cfg;
}
