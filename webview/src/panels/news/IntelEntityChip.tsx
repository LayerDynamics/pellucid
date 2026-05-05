import { type ReactElement } from "react";

import { cn } from "../../components/primitives/cn";

/**
 * Compact pill rendering one named entity surfaced by the
 * intelligence pipeline: a country, an actor, an organisation, a
 * person, an event, etc. Shared by every panel that lists a
 * dense set of entities (e.g. `GdeltIntelPanel`,
 * `RegionalIntelligenceBoard`, `CountryDeepDivePanel` evidence
 * lists).
 *
 * Props mirror the entity shape the intel handlers return — a
 * type tag plus a free-form name. The chip color-codes by type
 * so a long list can be skimmed visually.
 */

export type IntelEntityKind =
  | "country"
  | "region"
  | "actor"
  | "organisation"
  | "person"
  | "event"
  | "topic"
  | "weapon"
  | "incident";

export interface IntelEntity {
  /** Stable id (often `<kind>:<slug>` from upstream). */
  id: string;
  /** Entity kind — drives the chip color. */
  kind: IntelEntityKind;
  /** Display name. */
  name: string;
  /** Optional confidence score in [0, 1]. When set the chip shows
   *  a 1-decimal "76%" suffix. */
  confidence?: number;
  /** Optional source citation (URL, event id) — not rendered by
   *  the chip itself but carried so click handlers can route. */
  href?: string;
}

export interface IntelEntityChipProps {
  /** The entity to render. */
  entity: IntelEntity;
  /** Emit a click event with the entity. */
  onSelect?: (e: IntelEntity) => void;
  /** Extra Tailwind classes. */
  className?: string;
}

const KIND_CLASS: Record<IntelEntityKind, string> = {
  country:
    "border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] text-[var(--pellucid-fg)]",
  region:
    "border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] text-[var(--pellucid-fg)]",
  actor:
    "border-[color:var(--pellucid-warn)] text-[var(--pellucid-warn)]",
  organisation:
    "border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] text-[var(--pellucid-muted)]",
  person:
    "border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] text-[var(--pellucid-muted)]",
  event:
    "border-[color:var(--pellucid-info)] text-[var(--pellucid-info)]",
  topic:
    "border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] text-[var(--pellucid-muted)]",
  weapon:
    "border-[color:var(--pellucid-danger)] bg-[color:var(--pellucid-danger)]/10 text-[var(--pellucid-danger)]",
  incident:
    "border-[color:var(--pellucid-danger)] text-[var(--pellucid-danger)]",
};

const KIND_GLYPH: Record<IntelEntityKind, string> = {
  country: "🌐",
  region: "🗺",
  actor: "⚑",
  organisation: "▣",
  person: "◯",
  event: "★",
  topic: "#",
  weapon: "⌖",
  incident: "✦",
};

export function IntelEntityChip(props: IntelEntityChipProps): ReactElement {
  const { entity, onSelect, className } = props;
  const Tag = onSelect ? "button" : "span";
  const tagProps = onSelect
    ? {
        type: "button" as const,
        onClick: () => onSelect(entity),
      }
    : {};
  return (
    <Tag
      data-component="IntelEntityChip"
      data-entity-id={entity.id}
      data-entity-kind={entity.kind}
      aria-label={`${entity.kind}: ${entity.name}`}
      className={cn(
        "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] leading-none",
        onSelect && "cursor-pointer hover:opacity-80",
        KIND_CLASS[entity.kind],
        className,
      )}
      {...tagProps}
    >
      <span aria-hidden="true" className="text-[10px] opacity-70">
        {KIND_GLYPH[entity.kind]}
      </span>
      <span data-field="name">{entity.name}</span>
      {typeof entity.confidence === "number" ? (
        <span
          data-field="confidence"
          className="font-mono text-[10px] opacity-70"
        >
          {formatConfidence(entity.confidence)}
        </span>
      ) : null}
    </Tag>
  );
}

/**
 * Format a confidence score in [0, 1] as a "76%" string. Clamps
 * out-of-range inputs so a malformed upstream value doesn't
 * render `"NaN%"` or `"-30%"`.
 */
export function formatConfidence(score: number): string {
  if (!Number.isFinite(score)) return "?%";
  const clamped = Math.max(0, Math.min(1, score));
  return `${Math.round(clamped * 100)}%`;
}
