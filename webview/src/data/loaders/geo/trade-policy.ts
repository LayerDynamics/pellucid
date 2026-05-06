import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface TariffRow {
  authority: string;
  origin: string;
  destination: string;
  hsCode: string;
  product: string;
  rateDeltaPp: number;
  effective: string;
  headline: string;
}

export interface TradePolicyResponse {
  rows: TariffRow[];
  totalRateDeltaPp: number;
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type TradePolicyOutcome = Outcome<TradePolicyResponse>;

export async function loadTradePolicy(opts: LoadOptions = {}): Promise<TradePolicyOutcome> {
  return fetchOutcome<TradePolicyResponse>("/api/trade/v1/policy", opts);
}
