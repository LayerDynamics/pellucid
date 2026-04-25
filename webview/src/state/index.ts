/**
 * Zustand store registry — re-exports every Pellucid store so tests and
 * components can `import { useAuthStore, ... } from "@/state"`.
 *
 * Each store is its own file with a typed initial state, named actions,
 * and (where required) `persist` + `subscribeWithSelector` middleware.
 * Cross-store reactions live in `reactions.ts` and are wired by the
 * 8-phase boot machine in T1.11.
 */

export { useAuthStore } from "./useAuthStore";
export { useBootStore } from "./useBootStore";
export { useCorrelationStore } from "./useCorrelationStore";
export { useDataStore } from "./useDataStore";
export { useMapStore } from "./useMapStore";
export { useNewsStore } from "./useNewsStore";
export { usePanelStore } from "./usePanelStore";
export { useUiStore } from "./useUiStore";
export { useVariantStore } from "./useVariantStore";

export type { AuthState } from "./useAuthStore";
export type { BootPhase, BootState } from "./useBootStore";
export type { CorrelationResult, CorrelationState } from "./useCorrelationStore";
export type { DataState, PanelData } from "./useDataStore";
export type { MapMode, MapState } from "./useMapStore";
export type { NewsState } from "./useNewsStore";
export type { PanelLayout, PanelState } from "./usePanelStore";
export type { Theme, UiState } from "./useUiStore";
export type { Variant, VariantState } from "./useVariantStore";
