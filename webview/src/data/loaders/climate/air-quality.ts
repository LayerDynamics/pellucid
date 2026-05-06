import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface AirQualityRow {
  city: string;
  country: string;
  aqi: number;
  pollutant: string;
  value: number;
  unit: string;
}

export interface AirQualityResponse {
  rows: AirQualityRow[];
  worstAqi: number;
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type AirQualityOutcome = Outcome<AirQualityResponse>;

export async function loadAirQuality(opts: LoadOptions = {}): Promise<AirQualityOutcome> {
  return fetchOutcome<AirQualityResponse>("/api/climate/v1/air-quality", opts);
}
