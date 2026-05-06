import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface CampaignRow {
  id: string;
  title: string;
  actor: string;
  severity: string;
  sectors: string[];
}

export interface CampaignsResponse {
  rows: CampaignRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type CampaignsOutcome = Outcome<CampaignsResponse>;

export async function loadActiveCampaigns(opts: LoadOptions = {}): Promise<CampaignsOutcome> {
  return fetchOutcome<CampaignsResponse>("/api/infra/v1/active-campaigns", opts);
}
