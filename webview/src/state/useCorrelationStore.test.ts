import { afterEach, describe, expect, test } from "bun:test";

import { useCorrelationStore } from "./useCorrelationStore";

const result = (id: string, domain: "military" | "escalation" | "economic" | "disaster") => ({
  id,
  domain,
  score: 0.5,
  fingerprint: id,
  computedAtMs: 1000,
});

afterEach(() => useCorrelationStore.getState().clearResults());

describe("useCorrelationStore", () => {
  test("default active list is all four domains", () => {
    expect(useCorrelationStore.getState().active.length).toBe(4);
  });

  test("setActive deduplicates", () => {
    useCorrelationStore.getState().setActive(["military", "military", "economic"]);
    expect(useCorrelationStore.getState().active).toEqual(["military", "economic"]);
  });

  test("toggleDomain flips presence", () => {
    useCorrelationStore.getState().setActive(["military"]);
    useCorrelationStore.getState().toggleDomain("escalation");
    expect(useCorrelationStore.getState().active).toContain("escalation");
    useCorrelationStore.getState().toggleDomain("escalation");
    expect(useCorrelationStore.getState().active).not.toContain("escalation");
  });

  test("setResult records by id and updates lastRunAt", () => {
    useCorrelationStore.getState().setResult(result("r1", "military"));
    expect(useCorrelationStore.getState().results.r1?.id).toBe("r1");
    expect(useCorrelationStore.getState().lastRunAtMs).toBe(1000);
  });

  test("resultsForDomain filters", () => {
    useCorrelationStore.getState().setResult(result("a", "military"));
    useCorrelationStore.getState().setResult(result("b", "economic"));
    expect(useCorrelationStore.getState().resultsForDomain("military").length).toBe(1);
    expect(useCorrelationStore.getState().resultsForDomain("economic").length).toBe(1);
    expect(useCorrelationStore.getState().resultsForDomain("disaster").length).toBe(0);
  });

  test("clearResults wipes results and lastRunAt", () => {
    useCorrelationStore.getState().setResult(result("a", "military"));
    useCorrelationStore.getState().clearResults();
    expect(useCorrelationStore.getState().results).toEqual({});
    expect(useCorrelationStore.getState().lastRunAtMs).toBeNull();
  });
});
