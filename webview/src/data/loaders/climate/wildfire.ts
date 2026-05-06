import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface WildfireRow {
  label: string;
  region: string;
  lat: number;
  lon: number;
  acresBurned: number;
  containmentPct: number;
}

export interface WildfireResponse {
  rows: WildfireRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type WildfireOutcome = Outcome<WildfireResponse>;

export async function loadWildfire(opts: LoadOptions = {}): Promise<WildfireOutcome> {
  return fetchOutcome<WildfireResponse>("/api/climate/v1/wildfire", opts);
}
