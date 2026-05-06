import { fetchOutcome, type LoadOptions, type Outcome } from "./_shared";

export interface EventTile {
  kind: string;
  label: string;
  region: string;
  severity: string;
}

export interface NaturalEventsResponse {
  tiles: EventTile[];
  totalEvents: number;
  assembledAtMs: number;
  stale: boolean;
}

export type NaturalEventsOutcome = Outcome<NaturalEventsResponse>;

export async function loadNaturalEvents(opts: LoadOptions = {}): Promise<NaturalEventsOutcome> {
  return fetchOutcome<NaturalEventsResponse>("/api/climate/v1/natural-events", opts);
}
