import { type ReactElement, useState } from "react";

export interface AppProps {
  initialMessage?: string;
}

/**
 * Root component of the Pellucid webview.
 *
 * At T0.4 it is a minimal smoke target — Vite + React 19 boots, mounts
 * `<App>`, and the screen says "Pellucid". Subsequent tasks (T1.5 onward)
 * progressively replace this body with the Zustand-driven 8-phase boot
 * machine, the panel grid, and the maps.
 */
export function App({ initialMessage = "Pellucid" }: AppProps): ReactElement {
  const [message] = useState(initialMessage);

  return (
    <main data-testid="app-root">
      <h1 data-testid="app-title">{message}</h1>
      <p data-testid="app-tagline">Real-time situational awareness console.</p>
    </main>
  );
}
