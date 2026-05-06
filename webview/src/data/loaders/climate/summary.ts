import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface SummaryTile {
  code: string;
  label: string;
  value: string;
  tone: string;
}

export interface ClimateSummaryResponse {
  tiles: SummaryTile[];
  availableTiles: number;
  assembledAtMs: number;
  stale: boolean;
}

export type SummaryOutcome = Outcome<ClimateSummaryResponse>;

export async function loadClimateSummary(opts: LoadOptions = {}): Promise<SummaryOutcome> {
  return fetchOutcome<ClimateSummaryResponse>("/api/climate/v1/summary", opts);
}
