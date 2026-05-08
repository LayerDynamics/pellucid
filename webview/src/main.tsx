import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import {
  getApiBaseUrl,
  isDesktopRuntime,
  resolveLocalApiPort,
  toApiUrl,
} from "./services/runtime";
import "./styles/globals.css";

const rootElement = document.getElementById("root");
if (rootElement === null) {
  throw new Error("Pellucid: #root element missing from index.html");
}

// Desktop e2e bridge — `e2e/desktop/url-build.spec.ts` (and any future
// desktop spec wanting to inspect the URL-builder layer) needs a way to
// reach the runtime helpers from inside the bundled webview. Dynamic
// `import("/src/services/runtime.ts")` only works in vite dev because
// the production build emits content-hashed chunks; the spec was hitting
// a 404 inside `wry`. Expose a stable handle so specs can read the
// helpers via `window.__pellucidRuntime` without depending on bundle
// shape. Helpers are non-sensitive (pure URL composition + an IPC port
// lookup), so we attach unconditionally.
declare global {
  interface Window {
    __pellucidRuntime?: {
      isDesktopRuntime: typeof isDesktopRuntime;
      resolveLocalApiPort: typeof resolveLocalApiPort;
      getApiBaseUrl: typeof getApiBaseUrl;
      toApiUrl: typeof toApiUrl;
    };
  }
}
window.__pellucidRuntime = {
  isDesktopRuntime,
  resolveLocalApiPort,
  getApiBaseUrl,
  toApiUrl,
};

createRoot(rootElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
