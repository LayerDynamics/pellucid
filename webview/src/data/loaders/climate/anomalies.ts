import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface StationRecordRow {
  stationId: string;
  label: string;
  recordClass: string;
  value: number;
  setOn: string;
}

export interface AnomaliesResponse {
  globalAnomalyC?: number;
  period?: string;
  records: StationRecordRow[];
  assembledAtMs: number;
  stale: boolean;
}

export type AnomaliesOutcome = Outcome<AnomaliesResponse>;

export async function loadClimateAnomalies(opts: LoadOptions = {}): Promise<AnomaliesOutcome> {
  return fetchOutcome<AnomaliesResponse>("/api/climate/v1/anomalies", opts);
}
