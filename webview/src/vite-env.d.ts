/// <reference types="vite/client" />

declare const __VITE_TARGET__: "web" | "desktop";
declare const __VITE_VARIANT__: "base" | "tech" | "finance" | "commodity" | "happy";

interface ImportMetaEnv {
  readonly VITE_TARGET?: "web" | "desktop";
  readonly VITE_VARIANT?: "base" | "tech" | "finance" | "commodity" | "happy";
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
