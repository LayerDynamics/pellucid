import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface StrategicRiskResponse {
  level: string;
  score: number;
  headline: string;
  rationale: string;
  ucdpFatalities24h?: number;
  highReadinessTheaters?: number;
  sanctions24h?: number;
  assembledAtMs: number;
  stale: boolean;
}

export type StrategicRiskOutcome = Outcome<StrategicRiskResponse>;

export async function loadStrategicRisk(opts: LoadOptions = {}): Promise<StrategicRiskOutcome> {
  return fetchOutcome<StrategicRiskResponse>("/api/military/v1/strategic-risk", opts);
}
