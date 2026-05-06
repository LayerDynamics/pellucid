/**
 * SettingsModal — theme, language, panel-layout reset, current tier
 * display. Entirely client-side; persists via useUiStore.
 */

import { useCallback, type ReactElement } from "react";

import { Dialog, DialogContent } from "../components/primitives/Dialog";
import { Button } from "../components/primitives/Button";
import { useUiStore, type Theme } from "../state/useUiStore";
import { useAuthStore } from "../state/useAuthStore";
import { usePanelStore } from "../state/usePanelStore";

import { MODAL_SETTINGS } from "./index";

const THEME_OPTIONS: { value: Theme; label: string }[] = [
  { value: "system", label: "System" },
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
];

const LANG_OPTIONS = [
  { value: "en", label: "English" },
  { value: "es", label: "Español" },
  { value: "fr", label: "Français" },
  { value: "de", label: "Deutsch" },
];

export function SettingsModal(): ReactElement {
  const isOpen = useUiStore((s) => s.isModalOpen(MODAL_SETTINGS));
  const popModal = useUiStore((s) => s.popModal);
  const theme = useUiStore((s) => s.theme);
  const lang = useUiStore((s) => s.lang);
  const setTheme = useUiStore((s) => s.setTheme);
  const setLang = useUiStore((s) => s.setLang);
  const entitlements = useAuthStore((s) => s.entitlements);
  const email = useAuthStore((s) => s.email);
  const resetPanels = usePanelStore((s) => s.reset);

  const onResetLayout = useCallback(() => {
    resetPanels();
  }, [resetPanels]);

  const tierLabel =
    entitlements?.tier !== undefined
      ? ["Free", "Pro", "API", "Enterprise"][entitlements.tier] ?? `Tier ${entitlements.tier}`
      : "Signed out";

  return (
    <Dialog open={isOpen} onOpenChange={(o) => { if (!o) popModal(); }}>
      <DialogContent
        data-modal-id={MODAL_SETTINGS}
        heading="Settings"
        blurb="Theme, language, and dashboard layout."
        className="w-[480px]"
      >
        <div className="mt-3 flex flex-col gap-4">
          <section data-field="theme" className="flex flex-col gap-2">
            <span className="text-[10px] uppercase font-mono text-[var(--pellucid-fg-muted)]">Theme</span>
            <div className="flex gap-2">
              {THEME_OPTIONS.map((opt) => (
                <Button
                  key={opt.value}
                  type="button"
                  size="sm"
                  variant={theme === opt.value ? "solid" : "ghost"}
                  onClick={() => setTheme(opt.value)}
                  data-theme-option={opt.value}
                >
                  {opt.label}
                </Button>
              ))}
            </div>
          </section>
          <section data-field="lang" className="flex flex-col gap-2">
            <span className="text-[10px] uppercase font-mono text-[var(--pellucid-fg-muted)]">Language</span>
            <select
              value={lang}
              onChange={(e) => setLang(e.target.value)}
              data-field="lang-select"
              className="rounded border border-[var(--pellucid-border)] bg-transparent px-2 py-1.5 text-sm focus:outline-none focus:border-[var(--pellucid-info)]"
            >
              {LANG_OPTIONS.map((l) => (
                <option key={l.value} value={l.value}>{l.label}</option>
              ))}
            </select>
          </section>
          <section data-field="account" className="flex flex-col gap-2">
            <span className="text-[10px] uppercase font-mono text-[var(--pellucid-fg-muted)]">Account</span>
            <div className="rounded border border-[var(--pellucid-border)] p-2 text-xs">
              <div data-field="email">{email ?? "Not signed in"}</div>
              <div data-field="tier" className="text-[var(--pellucid-fg-muted)]">Tier: {tierLabel}</div>
            </div>
          </section>
          <section data-field="layout" className="flex flex-col gap-2">
            <span className="text-[10px] uppercase font-mono text-[var(--pellucid-fg-muted)]">Layout</span>
            <Button
              type="button"
              variant="ghost"
              onClick={onResetLayout}
              data-field="reset-layout"
            >
              Reset dashboard layout
            </Button>
          </section>
          <div className="flex justify-end pt-2">
            <Button type="button" variant="solid" onClick={() => popModal()} data-field="close">
              Done
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
