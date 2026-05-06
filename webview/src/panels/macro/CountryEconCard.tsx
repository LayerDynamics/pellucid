import { type ReactElement } from "react";

import { cn } from "../../components/primitives/cn";

export interface CountryEconSummary {
  /** Country name. */
  country: string;
  /** Country ISO code. */
  iso?: string;
  /** GDP (latest, $B), if known. */
  gdpUsdBillion?: number;
  /** GDP YoY % growth. */
  gdpYoyPct?: number;
  /** Inflation YoY %. */
  inflationYoyPct?: number;
  /** Unemployment %. */
  unemploymentPct?: number;
  /** Policy rate %. */
  policyRatePct?: number;
}

export interface CountryEconCardProps {
  /** Country summary to render. */
  summary: CountryEconSummary;
  /** Click handler. */
  onSelect?: (s: CountryEconSummary) => void;
  /** Extra Tailwind classes. */
  className?: string;
}

export function CountryEconCard(props: CountryEconCardProps): ReactElement {
  const { summary, onSelect, className } = props;
  const Tag = onSelect ? "button" : "article";
  return (
    <Tag
      data-component="CountryEconCard"
      data-country={summary.country}
      data-iso={summary.iso ?? ""}
      onClick={onSelect ? () => onSelect(summary) : undefined}
      className={cn(
        "flex flex-col gap-1 rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] p-3 text-left",
        onSelect ? "cursor-pointer hover:border-[var(--pellucid-info)]" : "",
        className,
      )}
    >
      <header className="flex items-baseline justify-between">
        <h4 data-field="country" className="text-sm font-semibold">
          {summary.country}
        </h4>
        {summary.iso ? (
          <span data-field="iso" className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">
            {summary.iso}
          </span>
        ) : null}
      </header>
      <dl className="grid grid-cols-2 gap-1 text-[11px]">
        {typeof summary.gdpUsdBillion === "number" ? (
          <Stat label="GDP" value={`$${summary.gdpUsdBillion.toFixed(0)}B`} dataField="gdp" />
        ) : null}
        {typeof summary.gdpYoyPct === "number" ? (
          <Stat
            label="GDP YoY"
            value={`${summary.gdpYoyPct >= 0 ? "+" : ""}${summary.gdpYoyPct.toFixed(1)}%`}
            tone={summary.gdpYoyPct >= 0 ? "positive" : "negative"}
            dataField="gdp-yoy"
          />
        ) : null}
        {typeof summary.inflationYoyPct === "number" ? (
          <Stat
            label="Inflation"
            value={`${summary.inflationYoyPct >= 0 ? "+" : ""}${summary.inflationYoyPct.toFixed(1)}%`}
            tone={summary.inflationYoyPct > 4 ? "negative" : "neutral"}
            dataField="inflation"
          />
        ) : null}
        {typeof summary.unemploymentPct === "number" ? (
          <Stat
            label="Unemployment"
            value={`${summary.unemploymentPct.toFixed(1)}%`}
            tone={summary.unemploymentPct > 6 ? "negative" : "neutral"}
            dataField="unemployment"
          />
        ) : null}
        {typeof summary.policyRatePct === "number" ? (
          <Stat
            label="Policy rate"
            value={`${summary.policyRatePct.toFixed(2)}%`}
            dataField="policy-rate"
          />
        ) : null}
      </dl>
    </Tag>
  );
}

function Stat({
  label,
  value,
  tone,
  dataField,
}: {
  label: string;
  value: string;
  tone?: "positive" | "negative" | "neutral";
  dataField: string;
}): ReactElement {
  const cls = tone === "positive"
    ? "text-[var(--pellucid-success)]"
    : tone === "negative"
      ? "text-[var(--pellucid-danger)]"
      : "text-[var(--pellucid-fg)]";
  return (
    <div data-component="CountryEconStat" data-field={dataField} className="flex flex-col">
      <dt className="text-[10px] uppercase font-mono text-[var(--pellucid-muted)]">{label}</dt>
      <dd className={`font-mono ${cls}`}>{value}</dd>
    </div>
  );
}
