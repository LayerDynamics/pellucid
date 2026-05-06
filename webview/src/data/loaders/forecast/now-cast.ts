import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface ForecastRow {
  id: string;
  question: string;
  probability: number;
  trend: string;
}

export interface NowCastResponse {
  rows: ForecastRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type NowCastOutcome = Outcome<NowCastResponse>;

export async function loadNowCast(opts: LoadOptions = {}): Promise<NowCastOutcome> {
  return fetchOutcome<NowCastResponse>("/api/forecast/v1/now-cast", opts);
}
