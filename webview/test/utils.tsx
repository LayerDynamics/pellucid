/**
 * Shared testing-library utilities for unit + integration tests.
 *
 * `renderApp` wraps `<App>` (or any node) in the same StrictMode +
 * (eventual) Zustand provider stack used in production. Subsequent tasks
 * (T1.5 store split, T1.6 Radix theme provider) compose into this.
 */

import { type ReactElement, type ReactNode, StrictMode } from "react";
import { render, type RenderOptions, type RenderResult } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";

export type ProviderProps = {
  children: ReactNode;
};

/**
 * Wraps the rendered tree in the production provider stack. At T0.7 the
 * stack is just StrictMode; T1.5 adds Zustand, T1.6 adds Radix theme.
 * Tests should always go through this so they exercise the same component
 * tree the real app does.
 */
export function AllProviders({ children }: ProviderProps): ReactElement {
  return <StrictMode>{children}</StrictMode>;
}

/**
 * Render a component inside the Pellucid provider stack and return the
 * RTL result alongside a pre-configured `userEvent` instance for input
 * interactions.
 */
export interface RenderAppResult extends RenderResult {
  user: UserEvent;
}

export function renderApp(
  ui: ReactElement,
  options: Omit<RenderOptions, "wrapper"> = {},
): RenderAppResult {
  const result = render(ui, { wrapper: AllProviders, ...options });
  const user = userEvent.setup();
  return { ...result, user };
}

/**
 * Async helper for assertions that depend on React 19 concurrent
 * scheduling. Resolves after the next microtask flush.
 */
export async function flush(): Promise<void> {
  await new Promise<void>((resolve) => {
    queueMicrotask(resolve);
  });
}
