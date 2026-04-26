import { forwardRef, type ComponentPropsWithoutRef } from "react";
import * as RadixSlider from "@radix-ui/react-slider";

import { cn } from "./cn";

export interface SliderProps
  extends ComponentPropsWithoutRef<typeof RadixSlider.Root> {
  thumbCount?: number;
}

export const Slider = forwardRef<HTMLSpanElement, SliderProps>(
  function Slider({ className, thumbCount = 1, ...rest }, ref) {
    const thumbs = Array.from({ length: thumbCount }, (_, index) => index);
    return (
      <RadixSlider.Root
        ref={ref}
        data-pellucid="slider-root"
        className={cn(
          "relative flex h-5 w-full touch-none select-none items-center",
          className,
        )}
        {...rest}
      >
        <RadixSlider.Track
          data-pellucid="slider-track"
          className="relative h-1 w-full grow rounded-full bg-[var(--pellucid-surface-raised)]"
        >
          <RadixSlider.Range
            data-pellucid="slider-range"
            className="absolute h-full rounded-full bg-[var(--pellucid-accent)]"
          />
        </RadixSlider.Track>
        {thumbs.map((index) => (
          <RadixSlider.Thumb
            key={index}
            data-pellucid="slider-thumb"
            data-thumb-index={index}
            className="block h-4 w-4 rounded-full border border-[var(--pellucid-border)] bg-[var(--pellucid-fg)] shadow-[var(--pellucid-shadow-panel)] focus:outline-none"
          />
        ))}
      </RadixSlider.Root>
    );
  },
);
