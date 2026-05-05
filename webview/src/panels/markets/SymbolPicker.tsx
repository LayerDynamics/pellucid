import { useEffect, useState, type ReactElement } from "react";

import { cn } from "../../components/primitives/cn";

export interface SymbolPickerProps {
  /** Currently-selected symbol (controlled). */
  value: string;
  /** Fired when the user commits a symbol — typing alone does
   *  not commit; the user presses Enter, blurs, or clicks a
   *  preset chip. */
  onChange: (next: string) => void;
  /** Optional preset basket the picker exposes as quick-select
   *  chips beneath the input. */
  presets?: string[];
  /** Optional placeholder text. */
  placeholder?: string;
  /** Extra Tailwind classes on the wrapper. */
  className?: string;
  /** Optional label rendered before the input. Defaults to
   *  "Symbol". */
  label?: string;
}

/**
 * Compact symbol picker — text input + preset chips. The market
 * panels reuse this anywhere a user picks a single ticker
 * (StockAnalysisPanel, StockBacktestPanel, etc.).
 *
 * The component is intentionally agnostic about validation:
 * symbols are uppercased on commit and trimmed; downstream
 * code is responsible for resolving them. Empty commits are
 * dropped silently so the picker never "selects" an empty
 * string.
 */
export function SymbolPicker(props: SymbolPickerProps): ReactElement {
  const {
    value,
    onChange,
    presets = [],
    placeholder,
    className,
    label = "Symbol",
  } = props;

  const [draft, setDraft] = useState<string>(value);

  // Keep the local draft in sync when the parent flips the
  // selected symbol via a preset click or external change.
  // Skip the sync while the input is focused so live typing
  // isn't clobbered by an in-flight controlled value update.
  useEffect(() => {
    if (document.activeElement?.tagName !== "INPUT") {
      setDraft(value);
    }
  }, [value]);

  const commit = (raw: string) => {
    const next = raw.trim().toUpperCase();
    if (next.length === 0) return;
    setDraft(next);
    onChange(next);
  };

  return (
    <div
      data-component="SymbolPicker"
      className={cn("flex flex-col gap-1", className)}
    >
      <label className="flex items-center gap-1 text-[11px] uppercase font-mono text-[var(--pellucid-muted)]">
        <span>{label}</span>
        <input
          type="text"
          value={draft}
          aria-label={label}
          placeholder={placeholder ?? "AAPL"}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              commit(draft);
            }
          }}
          onBlur={(e) => commit(e.target.value)}
          className="rounded border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] px-2 py-0.5 text-[11px] uppercase text-[var(--pellucid-fg)] focus:outline-none focus:border-[var(--pellucid-info)]"
        />
      </label>
      {presets.length > 0 ? (
        <div
          role="toolbar"
          aria-label="Symbol presets"
          className="flex flex-wrap items-center gap-1"
        >
          {presets.map((p) => (
            <button
              key={p}
              type="button"
              data-symbol-preset={p}
              aria-pressed={value.toUpperCase() === p.toUpperCase() ? "true" : "false"}
              onClick={() => commit(p)}
              className="rounded-full border border-[var(--pellucid-border)] px-2 py-0.5 text-[11px] font-mono uppercase hover:opacity-80"
            >
              {p}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
