import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface ExtendedRow {
  id: string;
  question: string;
  probability: number;
  horizon: string;
  delta7d: number;
}

export interface ExtendedResponse {
  rows: ExtendedRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type ExtendedOutcome = Outcome<ExtendedResponse>;

export async function loadExtendedForecast(opts: LoadOptions = {}): Promise<ExtendedOutcome> {
  return fetchOutcome<ExtendedResponse>("/api/forecast/v1/extended", opts);
}
