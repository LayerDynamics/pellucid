import { create } from "zustand";
import { persist, subscribeWithSelector } from "zustand/middleware";

export type Theme = "system" | "dark" | "light";

export interface UiState {
  theme: Theme;
  lang: string;
  sidebarOpen: boolean;
  modalStack: string[];
  setTheme: (theme: Theme) => void;
  setLang: (lang: string) => void;
  toggleSidebar: () => void;
  setSidebar: (open: boolean) => void;
  pushModal: (id: string) => void;
  popModal: () => string | null;
  topModal: () => string | null;
  isModalOpen: (id: string) => boolean;
}

export const useUiStore = create<UiState>()(
  persist(
    subscribeWithSelector((set, get) => ({
      theme: "system",
      lang: "en",
      sidebarOpen: false,
      modalStack: [],
      setTheme: (theme) => set({ theme }),
      setLang: (lang) => set({ lang }),
      toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen })),
      setSidebar: (open) => set({ sidebarOpen: open }),
      pushModal: (id) => set((s) => ({ modalStack: [...s.modalStack, id] })),
      popModal: () => {
        const stack = get().modalStack;
        if (stack.length === 0) return null;
        const top = stack[stack.length - 1] ?? null;
        set({ modalStack: stack.slice(0, -1) });
        return top;
      },
      topModal: () => {
        const stack = get().modalStack;
        return stack[stack.length - 1] ?? null;
      },
      isModalOpen: (id) => get().modalStack.includes(id),
    })),
    {
      name: "pellucid:ui",
      partialize: (s) => ({ theme: s.theme, lang: s.lang, sidebarOpen: s.sidebarOpen }),
    },
  ),
);
