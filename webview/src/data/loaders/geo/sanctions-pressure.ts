import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface SanctionRow {
  authority: string;
  entity: string;
  entityType: string;
  jurisdiction: string;
  listedOn: string;
  programme: string;
}

export interface SanctionsPressureResponse {
  rows: SanctionRow[];
  byAuthority: Array<[string, number]>;
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type SanctionsPressureOutcome = Outcome<SanctionsPressureResponse>;

export async function loadSanctionsPressure(opts: LoadOptions = {}): Promise<SanctionsPressureOutcome> {
  return fetchOutcome<SanctionsPressureResponse>("/api/sanctions/v1/pressure", opts);
}
