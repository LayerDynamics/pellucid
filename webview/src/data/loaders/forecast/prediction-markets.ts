import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface MarketRow {
  id: string;
  question: string;
  yes_price: number;
  volume_usd: number;
  category: string;
}

export interface PredictionMarketsResponse {
  rows: MarketRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type PredictionMarketsOutcome = Outcome<PredictionMarketsResponse>;

export async function loadPredictionMarkets(opts: LoadOptions = {}): Promise<PredictionMarketsOutcome> {
  return fetchOutcome<PredictionMarketsResponse>("/api/forecast/v1/prediction-markets", opts);
}
