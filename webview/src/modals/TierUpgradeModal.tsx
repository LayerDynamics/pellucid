/**
 * TierUpgradeModal — surfaces current entitlements and an upgrade
 * action. The upgrade flow is delegated to the host shell via the
 * `onUpgrade` callback so the modal stays Tauri/web agnostic.
 */

import { type ReactElement } from "react";

import { Dialog, DialogContent } from "../components/primitives/Dialog";
import { Button } from "../components/primitives/Button";
import { useAuthStore } from "../state/useAuthStore";
import { useUiStore } from "../state/useUiStore";

import { MODAL_TIER } from "./index";

export interface TierDescriptor {
  tier: number;
  label: string;
  blurb: string;
  features: string[];
}

export const TIERS: TierDescriptor[] = [
  {
    tier: 0,
    label: "Free",
    blurb: "Single dashboard, base tier.",
    features: ["1 dashboard", "Base data refresh", "Community support"],
  },
  {
    tier: 1,
    label: "Pro",
    blurb: "Up to 4 dashboards + faster refresh.",
    features: ["4 dashboards", "Faster refresh", "Email support"],
  },
  {
    tier: 2,
    label: "API",
    blurb: "Programmatic access + 50 req/min.",
    features: ["API key", "50 req/min", "Webhook events"],
  },
  {
    tier: 3,
    label: "Enterprise",
    blurb: "Unlimited dashboards + SLA.",
    features: ["Unlimited dashboards", "Priority support", "SAML SSO", "SLA"],
  },
];

export interface TierUpgradeModalProps {
  /** Host shell hook — receives the target tier when the user clicks Upgrade. */
  onUpgrade?: (targetTier: number) => Promise<void> | void;
}

export function TierUpgradeModal(props: TierUpgradeModalProps): ReactElement {
  const isOpen = useUiStore((s) => s.isModalOpen(MODAL_TIER));
  const popModal = useUiStore((s) => s.popModal);
  const entitlements = useAuthStore((s) => s.entitlements);
  const currentTier = entitlements?.tier ?? 0;

  return (
    <Dialog open={isOpen} onOpenChange={(o) => { if (!o) popModal(); }}>
      <DialogContent
        data-modal-id={MODAL_TIER}
        heading="Subscription tier"
        blurb="Compare tiers and upgrade."
        className="w-[640px]"
      >
        <ul className="mt-3 grid grid-cols-1 gap-2 md:grid-cols-2" aria-label={`${TIERS.length} subscription tiers`}>
          {TIERS.map((t) => {
            const isCurrent = t.tier === currentTier;
            return (
              <li
                key={t.tier}
                data-component="TierCard"
                data-tier={t.tier}
                data-current={isCurrent ? "true" : "false"}
                className={`rounded border p-3 text-xs ${isCurrent ? "border-[var(--pellucid-success)]" : "border-[var(--pellucid-border)]"}`}
              >
                <div className="flex items-baseline justify-between">
                  <span data-field="label" className="text-sm font-semibold">{t.label}</span>
                  {isCurrent ? <span data-field="current-badge" className="text-[10px] uppercase font-mono text-[var(--pellucid-success)]">Current</span> : null}
                </div>
                <p data-field="blurb" className="text-[var(--pellucid-fg-muted)]">{t.blurb}</p>
                <ul className="mt-2 flex flex-col gap-0.5" aria-label={`${t.label} features`}>
                  {t.features.map((f) => (
                    <li key={f} data-field="feature" className="text-[var(--pellucid-fg-muted)]">· {f}</li>
                  ))}
                </ul>
                {!isCurrent && t.tier > currentTier ? (
                  <div className="mt-3 flex justify-end">
                    <Button
                      type="button"
                      size="sm"
                      variant="solid"
                      onClick={() => { void props.onUpgrade?.(t.tier); }}
                      data-field="upgrade"
                      data-target-tier={t.tier}
                    >
                      Upgrade to {t.label}
                    </Button>
                  </div>
                ) : null}
              </li>
            );
          })}
        </ul>
        <div className="flex justify-end pt-3">
          <Button type="button" variant="ghost" onClick={() => popModal()} data-field="close">
            Close
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
