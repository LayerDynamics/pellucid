/**
 * AuthModal — sign-in dialog. Renders a Radix Dialog + email/token form
 * that calls useAuthStore.signIn on submit. Used at boot when the user
 * needs to authenticate against Clerk; the actual Clerk session token
 * exchange lives in the Tauri shell — this modal is the user-facing
 * entry point.
 */

import { useState, type FormEvent, type ReactElement } from "react";

import { Dialog, DialogContent } from "../components/primitives/Dialog";
import { Button } from "../components/primitives/Button";
import { useAuthStore } from "../state/useAuthStore";
import { useUiStore } from "../state/useUiStore";

import { MODAL_AUTH } from "./index";

export interface AuthModalProps {
  /**
   * Optional submit handler — receives the raw email/token pair so the
   * Tauri shell can perform the Clerk exchange. Defaults to a passthrough
   * that calls useAuthStore.signIn directly with the entered fields.
   */
  onSubmit?: (params: { email: string; token: string }) => Promise<void> | void;
}

export function AuthModal(props: AuthModalProps): ReactElement {
  const isOpen = useUiStore((s) => s.isModalOpen(MODAL_AUTH));
  const popModal = useUiStore((s) => s.popModal);
  const signIn = useAuthStore((s) => s.signIn);
  const [email, setEmail] = useState("");
  const [token, setToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(e: FormEvent<HTMLFormElement>): Promise<void> {
    e.preventDefault();
    if (!email.trim() || !token.trim()) {
      setError("Email and token are required.");
      return;
    }
    setError(null);
    setSubmitting(true);
    try {
      if (props.onSubmit) {
        await props.onSubmit({ email: email.trim(), token: token.trim() });
      } else {
        signIn({
          userId: email.trim(),
          email: email.trim(),
          clerkSessionToken: token.trim(),
          entitlements: null,
        });
      }
      popModal();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Dialog open={isOpen} onOpenChange={(o) => { if (!o) popModal(); }}>
      <DialogContent
        data-modal-id={MODAL_AUTH}
        heading="Sign in"
        blurb="Enter your Clerk session token to authenticate."
        className="w-[420px]"
      >
        <form onSubmit={handleSubmit} className="mt-3 flex flex-col gap-3" data-form="auth">
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-[var(--pellucid-fg-muted)]">Email</span>
            <input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              autoComplete="email"
              data-field="email"
              className="rounded border border-[var(--pellucid-border)] bg-transparent px-2 py-1.5 text-sm focus:outline-none focus:border-[var(--pellucid-info)]"
              required
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-[var(--pellucid-fg-muted)]">Clerk session token</span>
            <input
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              data-field="token"
              className="rounded border border-[var(--pellucid-border)] bg-transparent px-2 py-1.5 font-mono text-sm focus:outline-none focus:border-[var(--pellucid-info)]"
              required
            />
          </label>
          {error ? (
            <div role="alert" data-field="error" className="text-xs text-[var(--pellucid-danger)]">
              {error}
            </div>
          ) : null}
          <div className="flex justify-end gap-2 pt-1">
            <Button type="button" variant="ghost" onClick={() => popModal()} data-field="cancel">
              Cancel
            </Button>
            <Button type="submit" variant="solid" disabled={submitting} data-field="submit">
              {submitting ? "Signing in…" : "Sign in"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
