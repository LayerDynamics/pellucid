import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface VolcanoRow {
  id: string;
  name: string;
  country: string;
  status: string;
  lat: number;
  lon: number;
}

export interface VolcanoResponse {
  rows: VolcanoRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type VolcanoOutcome = Outcome<VolcanoResponse>;

export async function loadVolcanoActivity(opts: LoadOptions = {}): Promise<VolcanoOutcome> {
  return fetchOutcome<VolcanoResponse>("/api/climate/v1/volcano-activity", opts);
}
