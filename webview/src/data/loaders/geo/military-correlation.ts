import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface CorrelationRow {
  theater: string;
  readiness: string;
  headcount: number;
  fatalities24h: number;
  deployments: number;
  correlation: number;
}

export interface MilitaryCorrelationResponse {
  rows: CorrelationRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type MilitaryCorrelationOutcome = Outcome<MilitaryCorrelationResponse>;

export async function loadMilitaryCorrelation(opts: LoadOptions = {}): Promise<MilitaryCorrelationOutcome> {
  return fetchOutcome<MilitaryCorrelationResponse>("/api/military/v1/military-correlation", opts);
}
