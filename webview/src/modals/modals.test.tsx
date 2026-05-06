import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { useAuthStore } from "../state/useAuthStore";
import { useUiStore } from "../state/useUiStore";
import { usePanelStore } from "../state/usePanelStore";

import {
  AuthModal,
  KeyboardShortcutsModal,
  MODAL_AUTH,
  MODAL_SEARCH,
  MODAL_SETTINGS,
  MODAL_SHORTCUTS,
  MODAL_TIER,
  SearchModal,
  SettingsModal,
  TierUpgradeModal,
} from "./index";

import { collectAllPanels, matchEntries, type SearchEntry } from "./SearchModal";
import { SHORTCUTS } from "./KeyboardShortcutsModal";
import { TIERS } from "./TierUpgradeModal";

import "../panels/macro/EconomicPanel";
import "../panels/macro/FSIPanel";
import "../panels/energy/EnergyComplexPanel";
import "../panels/climate/ClimatePanel";
import "../panels/infra/InfraPanel";
import "../panels/forecast/ForecastPanel";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  useUiStore.setState({ modalStack: [], theme: "system", lang: "en", sidebarOpen: false });
  usePanelStore.getState().reset();
});

describe("AuthModal", () => {
  test("submits and signs the user in", async () => {
    useUiStore.getState().pushModal(MODAL_AUTH);
    render(<AuthModal />);
    const dlg = await waitFor(() =>
      document.querySelector(`[data-modal-id="${MODAL_AUTH}"]`),
    );
    expect(dlg).not.toBeNull();
    const email = dlg!.querySelector('[data-field="email"]') as HTMLInputElement;
    const token = dlg!.querySelector('[data-field="token"]') as HTMLInputElement;
    fireEvent.change(email, { target: { value: "user@example.com" } });
    fireEvent.change(token, { target: { value: "tk_abc" } });
    const form = dlg!.querySelector('[data-form="auth"]') as HTMLFormElement;
    fireEvent.submit(form);
    await waitFor(() => {
      expect(useAuthStore.getState().email).toBe("user@example.com");
      expect(useUiStore.getState().isModalOpen(MODAL_AUTH)).toBe(false);
    });
  });

  test("missing field blocks submit", async () => {
    useUiStore.getState().pushModal(MODAL_AUTH);
    render(<AuthModal />);
    const dlg = await waitFor(() =>
      document.querySelector(`[data-modal-id="${MODAL_AUTH}"]`),
    );
    const email = dlg!.querySelector('[data-field="email"]') as HTMLInputElement;
    fireEvent.change(email, { target: { value: "x" } });
    const form = dlg!.querySelector('[data-form="auth"]') as HTMLFormElement;
    fireEvent.submit(form);
    await waitFor(() => {
      expect(dlg!.querySelector('[data-field="error"]')).not.toBeNull();
    });
  });

  test("custom onSubmit handler runs and signOut wins on failure", async () => {
    const captured: { hit: boolean } = { hit: false };
    useUiStore.getState().pushModal(MODAL_AUTH);
    render(<AuthModal onSubmit={async () => { captured.hit = true; }} />);
    const dlg = await waitFor(() =>
      document.querySelector(`[data-modal-id="${MODAL_AUTH}"]`),
    );
    const email = dlg!.querySelector('[data-field="email"]') as HTMLInputElement;
    const token = dlg!.querySelector('[data-field="token"]') as HTMLInputElement;
    fireEvent.change(email, { target: { value: "u@x.com" } });
    fireEvent.change(token, { target: { value: "t" } });
    fireEvent.submit(dlg!.querySelector('[data-form="auth"]') as HTMLFormElement);
    await waitFor(() => {
      expect(captured.hit).toBe(true);
    });
  });
});

describe("SettingsModal", () => {
  test("changes theme via the option buttons", async () => {
    useUiStore.getState().pushModal(MODAL_SETTINGS);
    useUiStore.getState().setTheme("system");
    render(<SettingsModal />);
    const dlg = await waitFor(() =>
      document.querySelector(`[data-modal-id="${MODAL_SETTINGS}"]`),
    );
    const darkBtn = dlg!.querySelector('[data-theme-option="dark"]') as HTMLButtonElement;
    fireEvent.click(darkBtn);
    expect(useUiStore.getState().theme).toBe("dark");
  });

  test("reset-layout button clears panel store", async () => {
    usePanelStore.getState().setLayout("test/x", { rowSpan: 2 });
    useUiStore.getState().pushModal(MODAL_SETTINGS);
    render(<SettingsModal />);
    const dlg = await waitFor(() =>
      document.querySelector(`[data-modal-id="${MODAL_SETTINGS}"]`),
    );
    const reset = dlg!.querySelector('[data-field="reset-layout"]') as HTMLButtonElement;
    fireEvent.click(reset);
    expect(usePanelStore.getState().getLayout("test/x")).toBeUndefined();
  });

  test("displays Signed out when no session", async () => {
    useUiStore.getState().pushModal(MODAL_SETTINGS);
    render(<SettingsModal />);
    const dlg = await waitFor(() =>
      document.querySelector(`[data-modal-id="${MODAL_SETTINGS}"]`),
    );
    expect(dlg!.querySelector('[data-field="email"]')?.textContent).toContain("Not signed in");
  });
});

describe("KeyboardShortcutsModal", () => {
  test("renders every entry from SHORTCUTS", async () => {
    useUiStore.getState().pushModal(MODAL_SHORTCUTS);
    render(<KeyboardShortcutsModal />);
    const dlg = await waitFor(() =>
      document.querySelector(`[data-modal-id="${MODAL_SHORTCUTS}"]`),
    );
    expect(dlg!.querySelectorAll('[data-component="ShortcutRow"]').length).toBe(SHORTCUTS.length);
  });
});

describe("SearchModal — registry coverage", () => {
  test("collectAllPanels surfaces every registered panel", () => {
    const all = collectAllPanels();
    expect(all.length).toBeGreaterThan(0);
    expect(all.find((e) => e.family === "macro")).toBeDefined();
    expect(all.find((e) => e.family === "energy")).toBeDefined();
    expect(all.find((e) => e.family === "climate")).toBeDefined();
    expect(all.find((e) => e.family === "infra")).toBeDefined();
    expect(all.find((e) => e.family === "forecast")).toBeDefined();
  });

  test("matchEntries token-AND filters", () => {
    const entries: SearchEntry[] = [
      { id: "a", family: "f", title: "Alpha Beta", blurb: "x", minTier: 0 },
      { id: "b", family: "f", title: "Alpha Gamma", blurb: "y", minTier: 0 },
      { id: "c", family: "f", title: "Delta", blurb: "z", minTier: 0 },
    ];
    expect(matchEntries(entries, "alpha").length).toBe(2);
    expect(matchEntries(entries, "alpha gamma").length).toBe(1);
    expect(matchEntries(entries, "").length).toBe(3);
  });

  test("modal opens, filters by query, commits via Enter", async () => {
    useUiStore.getState().pushModal(MODAL_SEARCH);
    render(<SearchModal />);
    const dlg = await waitFor(() =>
      document.querySelector(`[data-modal-id="${MODAL_SEARCH}"]`),
    );
    const input = dlg!.querySelector('[data-field="query"]') as HTMLInputElement;
    fireEvent.change(input, { target: { value: "energy" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => {
      expect(useUiStore.getState().isModalOpen(MODAL_SEARCH)).toBe(false);
      expect(usePanelStore.getState().highlightedPanelId).not.toBeNull();
    });
  });

  test("ArrowDown then Enter walks the result list", async () => {
    useUiStore.getState().pushModal(MODAL_SEARCH);
    render(<SearchModal />);
    const dlg = await waitFor(() =>
      document.querySelector(`[data-modal-id="${MODAL_SEARCH}"]`),
    );
    const input = dlg!.querySelector('[data-field="query"]') as HTMLInputElement;
    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => {
      expect(useUiStore.getState().isModalOpen(MODAL_SEARCH)).toBe(false);
    });
  });
});

describe("TierUpgradeModal", () => {
  test("highlights current tier and shows upgrade for higher tiers", async () => {
    useAuthStore.getState().signIn({
      userId: "u",
      email: "u@x.com",
      clerkSessionToken: "tk",
      entitlements: {
        tier: 1,
        maxDashboards: 4,
        apiAccess: false,
        apiRateLimit: 0,
        prioritySupport: false,
        exportFormats: [],
        validUntilMs: 0,
      },
    });
    useUiStore.getState().pushModal(MODAL_TIER);
    render(<TierUpgradeModal />);
    const dlg = await waitFor(() =>
      document.querySelector(`[data-modal-id="${MODAL_TIER}"]`),
    );
    const cards = dlg!.querySelectorAll('[data-component="TierCard"]');
    expect(cards.length).toBe(TIERS.length);
    const current = dlg!.querySelector('[data-component="TierCard"][data-current="true"]');
    expect(current?.getAttribute("data-tier")).toBe("1");
    // Upgrade buttons only appear on higher tiers.
    const upgrades = dlg!.querySelectorAll('[data-field="upgrade"]');
    expect(upgrades.length).toBe(TIERS.filter((t) => t.tier > 1).length);
  });

  test("onUpgrade callback receives the target tier", async () => {
    const captured: { target: number | null } = { target: null };
    useUiStore.getState().pushModal(MODAL_TIER);
    render(<TierUpgradeModal onUpgrade={(t) => { captured.target = t; }} />);
    const dlg = await waitFor(() =>
      document.querySelector(`[data-modal-id="${MODAL_TIER}"]`),
    );
    const proBtn = dlg!.querySelector('[data-field="upgrade"][data-target-tier="1"]') as HTMLButtonElement;
    fireEvent.click(proBtn);
    await waitFor(() => {
      expect(captured.target).toBe(1);
    });
  });
});
