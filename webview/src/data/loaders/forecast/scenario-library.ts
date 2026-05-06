import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface ScenarioRow {
  id: string;
  title: string;
  domain: string;
  probability: number;
}

export interface ScenarioLibraryResponse {
  rows: ScenarioRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type ScenarioLibraryOutcome = Outcome<ScenarioLibraryResponse>;

export async function loadScenarioLibrary(opts: LoadOptions = {}): Promise<ScenarioLibraryOutcome> {
  return fetchOutcome<ScenarioLibraryResponse>("/api/forecast/v1/scenario-library", opts);
}
