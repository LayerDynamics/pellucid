import { useMemo, type ReactElement } from "react";

import { cn } from "../../components/primitives/cn";

/**
 * One OHLC candle. Times are wall-clock ms (UTC). Pure data —
 * no styling decisions live on the candle itself.
 */
export interface OhlcCandle {
  /** Wall-clock ms when the candle's bucket opens. */
  timeMs: number;
  /** Open price. */
  open: number;
  /** Highest traded price during the bucket. */
  high: number;
  /** Lowest traded price during the bucket. */
  low: number;
  /** Close price. */
  close: number;
}

export interface OhlcChartProps {
  /** Candles to plot. Sorted ascending by `timeMs` by the caller;
   *  the chart renders them in array order. */
  candles: OhlcCandle[];
  /** Render width in CSS pixels. Defaults to 480. */
  width?: number;
  /** Render height in CSS pixels. Defaults to 200. */
  height?: number;
  /** Optional title rendered above the chart. */
  title?: string;
  /** Extra Tailwind classes on the wrapper. */
  className?: string;
}

/**
 * SVG-based OHLC candlestick chart. Pure presentational — picks
 * a price scale from the candle min/low and high/max, draws a
 * thin wick from low→high and a 6-px-wide body from open→close.
 *
 * Up candles (close >= open) get the design-token `--pellucid-
 * success` colour; down candles get `--pellucid-danger`.
 *
 * Empty input renders an "—" placeholder so dashboards with no
 * OHLC data don't show a blank `<svg>`.
 */
export function OhlcChart(props: OhlcChartProps): ReactElement {
  const {
    candles,
    width = 480,
    height = 200,
    title,
    className,
  } = props;

  const layout = useMemo(() => buildLayout(candles, width, height), [
    candles,
    width,
    height,
  ]);

  if (candles.length === 0) {
    return (
      <div
        data-component="OhlcChart"
        data-state="empty"
        className={cn(
          "flex items-center justify-center rounded-md border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] text-xs text-[var(--pellucid-muted)]",
          className,
        )}
        style={{ width, height }}
      >
        — no candles —
      </div>
    );
  }

  return (
    <figure
      data-component="OhlcChart"
      data-state="ready"
      data-candle-count={candles.length}
      className={cn("flex flex-col gap-1", className)}
    >
      {title ? (
        <figcaption className="text-[11px] uppercase font-mono text-[var(--pellucid-muted)]">
          {title}
        </figcaption>
      ) : null}
      <svg
        role="img"
        aria-label={`${candles.length} OHLC candles${title ? `: ${title}` : ""}`}
        width={width}
        height={height}
        viewBox={`0 0 ${width} ${height}`}
        className="overflow-visible"
      >
        {layout.bars.map((bar) => (
          <g key={bar.timeMs} data-component="OhlcCandle" data-time-ms={bar.timeMs}>
            <line
              x1={bar.x}
              x2={bar.x}
              y1={bar.wickTop}
              y2={bar.wickBottom}
              stroke={bar.up ? "var(--pellucid-success)" : "var(--pellucid-danger)"}
              strokeWidth={1}
              data-field="wick"
            />
            <rect
              x={bar.x - 3}
              y={bar.bodyTop}
              width={6}
              height={Math.max(1, bar.bodyHeight)}
              fill={bar.up ? "var(--pellucid-success)" : "var(--pellucid-danger)"}
              data-field="body"
              data-up={bar.up ? "true" : "false"}
            />
          </g>
        ))}
      </svg>
    </figure>
  );
}

interface LayoutBar {
  timeMs: number;
  x: number;
  wickTop: number;
  wickBottom: number;
  bodyTop: number;
  bodyHeight: number;
  up: boolean;
}

interface ChartLayout {
  bars: LayoutBar[];
  priceMin: number;
  priceMax: number;
}

/**
 * Compute candle layout from raw candles. Pure — exported for
 * unit tests that pin the math.
 */
export function buildLayout(
  candles: OhlcCandle[],
  width: number,
  height: number,
): ChartLayout {
  if (candles.length === 0) {
    return { bars: [], priceMin: 0, priceMax: 0 };
  }
  const padX = 8;
  const padY = 8;
  const innerW = Math.max(1, width - padX * 2);
  const innerH = Math.max(1, height - padY * 2);
  const priceMin = candles.reduce((m, c) => Math.min(m, c.low), candles[0]!.low);
  const priceMax = candles.reduce((m, c) => Math.max(m, c.high), candles[0]!.high);
  const priceSpan = priceMax - priceMin || 1;
  const stepX = innerW / Math.max(1, candles.length - 1 + 1);

  const bars: LayoutBar[] = candles.map((c, i) => {
    const x = padX + stepX * (i + 0.5);
    const yFor = (price: number) =>
      padY + innerH * (1 - (price - priceMin) / priceSpan);
    const up = c.close >= c.open;
    const bodyTop = yFor(Math.max(c.open, c.close));
    const bodyBottom = yFor(Math.min(c.open, c.close));
    return {
      timeMs: c.timeMs,
      x,
      wickTop: yFor(c.high),
      wickBottom: yFor(c.low),
      bodyTop,
      bodyHeight: bodyBottom - bodyTop,
      up,
    };
  });
  return { bars, priceMin, priceMax };
}
