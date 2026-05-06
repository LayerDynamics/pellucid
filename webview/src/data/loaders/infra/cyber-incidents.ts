import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface IncidentRow {
  id: string;
  title: string;
  severity: string;
  source: string;
  publishedAtMs: number;
}

export interface CyberIncidentsResponse {
  rows: IncidentRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type CyberIncidentsOutcome = Outcome<CyberIncidentsResponse>;

export async function loadCyberIncidents(opts: LoadOptions = {}): Promise<CyberIncidentsOutcome> {
  return fetchOutcome<CyberIncidentsResponse>("/api/infra/v1/cyber-incidents", opts);
}
