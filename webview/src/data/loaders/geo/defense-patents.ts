import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface PatentClassRow {
  cpcClass: string;
  label: string;
  filings30d: number;
  yoyPct: number;
  topFiler: string;
}

export interface DefensePatentsResponse {
  rows: PatentClassRow[];
  totalFilings30d: number;
  period: string;
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type DefensePatentsOutcome = Outcome<DefensePatentsResponse>;

export async function loadDefensePatents(opts: LoadOptions = {}): Promise<DefensePatentsOutcome> {
  return fetchOutcome<DefensePatentsResponse>("/api/military/v1/defense-patents", opts);
}
