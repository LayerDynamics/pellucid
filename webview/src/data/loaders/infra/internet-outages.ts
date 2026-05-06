import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface OutageRow {
  provider: string;
  region: string;
  status: string;
  startedAtMs: number;
  affectedAsCount: number;
}

export interface OutagesResponse {
  rows: OutageRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type OutagesOutcome = Outcome<OutagesResponse>;

export async function loadInternetOutages(opts: LoadOptions = {}): Promise<OutagesOutcome> {
  return fetchOutcome<OutagesResponse>("/api/infra/v1/internet-outages", opts);
}
