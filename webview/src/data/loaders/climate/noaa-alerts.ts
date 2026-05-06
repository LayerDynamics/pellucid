import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface AlertRow {
  event: string;
  area: string;
  severity: string;
  urgency: string;
  effective: string;
  expires: string;
}

export interface NoaaAlertsResponse {
  rows: AlertRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type NoaaAlertsOutcome = Outcome<NoaaAlertsResponse>;

export async function loadNoaaAlerts(opts: LoadOptions = {}): Promise<NoaaAlertsOutcome> {
  return fetchOutcome<NoaaAlertsResponse>("/api/climate/v1/noaa-alerts", opts);
}
