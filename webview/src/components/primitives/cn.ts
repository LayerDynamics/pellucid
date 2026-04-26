import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Merge Tailwind class strings while resolving conflicts (last writer wins).
 * Used by every primitive wrapper so consumers can override the default
 * Pellucid styling with extra `className` prop without specificity wars.
 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
