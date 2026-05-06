import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface ThermalRow {
  id: string;
  lat: number;
  lon: number;
  brightnessK: number;
  confidence: number;
  zone: string;
  acquiredAt: string;
}

export interface ThermalEscalationResponse {
  rows: ThermalRow[];
  highConfidenceCount: number;
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type ThermalEscalationOutcome = Outcome<ThermalEscalationResponse>;

export async function loadThermalEscalation(opts: LoadOptions = {}): Promise<ThermalEscalationOutcome> {
  return fetchOutcome<ThermalEscalationResponse>("/api/thermal/v1/escalation", opts);
}
