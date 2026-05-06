import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface QuakeRow {
  id: string;
  place: string;
  mag: number;
  depth: number;
  lat: number;
  lon: number;
  occurredAtMs: number;
}

export interface EarthquakesResponse {
  rows: QuakeRow[];
  maxMag: number;
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type EarthquakesOutcome = Outcome<EarthquakesResponse>;

export async function loadEarthquakes(opts: LoadOptions = {}): Promise<EarthquakesOutcome> {
  return fetchOutcome<EarthquakesResponse>("/api/climate/v1/earthquakes", opts);
}
