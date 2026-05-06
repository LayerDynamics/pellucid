import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface ForecastTile {
  code: string;
  label: string;
  value: string;
  tone: string;
}

export interface ForecastSummaryResponse {
  tiles: ForecastTile[];
  availableTiles: number;
  assembledAtMs: number;
  stale: boolean;
}

export type ForecastSummaryOutcome = Outcome<ForecastSummaryResponse>;

export async function loadForecastSummary(opts: LoadOptions = {}): Promise<ForecastSummaryOutcome> {
  return fetchOutcome<ForecastSummaryResponse>("/api/forecast/v1/summary", opts);
}
