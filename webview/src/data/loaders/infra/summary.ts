import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface InfraTile {
  code: string;
  label: string;
  value: string;
  tone: string;
}

export interface InfraSummaryResponse {
  tiles: InfraTile[];
  availableTiles: number;
  assembledAtMs: number;
  stale: boolean;
}

export type InfraSummaryOutcome = Outcome<InfraSummaryResponse>;

export async function loadInfraSummary(opts: LoadOptions = {}): Promise<InfraSummaryOutcome> {
  return fetchOutcome<InfraSummaryResponse>("/api/infra/v1/summary", opts);
}
