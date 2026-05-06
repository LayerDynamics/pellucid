import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface PostureRow {
  theater: string;
  force: string;
  readiness: string;
  headcount: number;
}

export interface StrategicPostureResponse {
  rows: PostureRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type StrategicPostureOutcome = Outcome<StrategicPostureResponse>;

export async function loadStrategicPosture(opts: LoadOptions = {}): Promise<StrategicPostureOutcome> {
  return fetchOutcome<StrategicPostureResponse>("/api/military/v1/strategic-posture", opts);
}
