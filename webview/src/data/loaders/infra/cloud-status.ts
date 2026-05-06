import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface CloudStatusRow {
  provider: string;
  component: string;
  status: string;
  region: string;
}

export interface CloudStatusResponse {
  rows: CloudStatusRow[];
  incidentCount: number;
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type CloudStatusOutcome = Outcome<CloudStatusResponse>;

export async function loadCloudStatus(opts: LoadOptions = {}): Promise<CloudStatusOutcome> {
  return fetchOutcome<CloudStatusResponse>("/api/infra/v1/cloud-status", opts);
}
