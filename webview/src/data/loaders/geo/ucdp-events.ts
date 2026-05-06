import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface UcdpEventRow {
  id: string;
  country: string;
  actor1: string;
  actor2: string;
  fatalities: number;
  lat: number;
  lon: number;
  occurredAt: string;
}

export interface UcdpEventsResponse {
  rows: UcdpEventRow[];
  totalFatalities: number;
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type UcdpEventsOutcome = Outcome<UcdpEventsResponse>;

export async function loadUcdpEvents(opts: LoadOptions = {}): Promise<UcdpEventsOutcome> {
  return fetchOutcome<UcdpEventsResponse>("/api/conflict/v1/ucdp-events", opts);
}
