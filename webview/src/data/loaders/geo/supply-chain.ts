import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface SupplyChainTile {
  code: string;
  label: string;
  value: string;
  tone: string;
}

export interface SupplyChainResponse {
  tiles: SupplyChainTile[];
  availableTiles: number;
  assembledAtMs: number;
  stale: boolean;
}

export type SupplyChainOutcome = Outcome<SupplyChainResponse>;

export async function loadSupplyChain(opts: LoadOptions = {}): Promise<SupplyChainOutcome> {
  return fetchOutcome<SupplyChainResponse>("/api/supply-chain/v1/summary", opts);
}
