import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface EscalationRow {
  zone: string;
  thermalAnomalies: number;
  fatalities24h: number;
  escalation: number;
}

export interface EscalationCorrelationResponse {
  rows: EscalationRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type EscalationCorrelationOutcome = Outcome<EscalationCorrelationResponse>;

export async function loadEscalationCorrelation(opts: LoadOptions = {}): Promise<EscalationCorrelationOutcome> {
  return fetchOutcome<EscalationCorrelationResponse>("/api/conflict/v1/escalation-correlation", opts);
}
