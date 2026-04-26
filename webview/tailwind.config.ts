import type { Config } from "tailwindcss";

const config: Config = {
  content: [
    "./index.html",
    "./src/**/*.{ts,tsx,css}",
  ],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        surface: "var(--pellucid-surface)",
        "surface-raised": "var(--pellucid-surface-raised)",
        accent: "var(--pellucid-accent)",
        "accent-fg": "var(--pellucid-accent-fg)",
        "accent-muted": "var(--pellucid-accent-muted)",
        fg: "var(--pellucid-fg)",
        "fg-muted": "var(--pellucid-fg-muted)",
        border: "var(--pellucid-border)",
        danger: "var(--pellucid-danger)",
        success: "var(--pellucid-success)",
      },
      borderRadius: {
        sm: "var(--pellucid-radius-sm)",
        md: "var(--pellucid-radius-md)",
        lg: "var(--pellucid-radius-lg)",
      },
      fontFamily: {
        sans: ["var(--pellucid-font-sans)", "system-ui", "sans-serif"],
        mono: ["var(--pellucid-font-mono)", "ui-monospace", "monospace"],
      },
      boxShadow: {
        panel: "var(--pellucid-shadow-panel)",
        focus: "0 0 0 2px var(--pellucid-accent-muted)",
      },
    },
  },
  plugins: [],
};

export default config;
