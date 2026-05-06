import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface StateRow {
  id: string;
  question: string;
  yesPrice: number;
  volumeUsd: number;
  category: string;
}

export interface ScenarioStateResponse {
  rows: StateRow[];
  topRow?: StateRow;
  weightedAvgYes: number;
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type ScenarioStateOutcome = Outcome<ScenarioStateResponse>;

export async function loadScenarioState(opts: LoadOptions = {}): Promise<ScenarioStateOutcome> {
  return fetchOutcome<ScenarioStateResponse>("/api/forecast/v1/scenario-state", opts);
}
