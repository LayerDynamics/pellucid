import { describe, expect, test } from "bun:test";

import {
  coerceVariant,
  detectVariantFrom,
  isVariant,
  matchHostnamePrefix,
  type DetectionSources,
} from "./variant";
import { ALL_VARIANTS } from "../state/useVariantStore";

const empty: DetectionSources = {
  fromBuild: null,
  fromHostname: null,
  fromStore: null,
};

describe("isVariant", () => {
  test("recognises every known variant id", () => {
    for (const v of ALL_VARIANTS) {
      expect(isVariant(v)).toBe(true);
    }
  });

  test("rejects unknown strings + non-strings", () => {
    expect(isVariant("BASE")).toBe(false); // case-sensitive
    expect(isVariant("")).toBe(false);
    expect(isVariant("unknown")).toBe(false);
    expect(isVariant(null)).toBe(false);
    expect(isVariant(undefined)).toBe(false);
    expect(isVariant(123)).toBe(false);
    expect(isVariant({})).toBe(false);
  });
});

describe("coerceVariant", () => {
  test("returns the variant for known ids", () => {
    expect(coerceVariant("tech")).toBe("tech");
    expect(coerceVariant("base")).toBe("base");
  });

  test("returns null for unknown / null", () => {
    expect(coerceVariant("unknown")).toBeNull();
    expect(coerceVariant(null)).toBeNull();
    expect(coerceVariant("")).toBeNull();
  });
});

describe("matchHostnamePrefix", () => {
  test("matches the expected prefix for each non-base variant", () => {
    expect(matchHostnamePrefix("tech.worldmonitor.app")).toBe("tech");
    expect(matchHostnamePrefix("finance.worldmonitor.app")).toBe("finance");
    expect(matchHostnamePrefix("commodity.worldmonitor.app")).toBe("commodity");
    expect(matchHostnamePrefix("happy.worldmonitor.app")).toBe("happy");
  });

  test("returns null for the bare domain", () => {
    // `base` has hostnamePrefix=null, so it MUST NOT be matched.
    expect(matchHostnamePrefix("worldmonitor.app")).toBeNull();
    expect(matchHostnamePrefix("www.worldmonitor.app")).toBeNull();
  });

  test("is case-insensitive on the hostname", () => {
    expect(matchHostnamePrefix("TECH.worldmonitor.app")).toBe("tech");
    expect(matchHostnamePrefix("Finance.WorldMonitor.app")).toBe("finance");
  });

  test("returns null for empty / null hostname", () => {
    expect(matchHostnamePrefix("")).toBeNull();
    expect(matchHostnamePrefix(null)).toBeNull();
  });

  test("does not match unrelated subdomains", () => {
    expect(matchHostnamePrefix("api.worldmonitor.app")).toBeNull();
    expect(matchHostnamePrefix("staging.worldmonitor.app")).toBeNull();
  });
});

describe("detectVariantFrom — priority order", () => {
  test("build wins over hostname + store", () => {
    expect(
      detectVariantFrom({
        fromBuild: "tech",
        fromHostname: "finance.worldmonitor.app",
        fromStore: "commodity",
      }),
    ).toBe("tech");
  });

  test("hostname wins when build is null", () => {
    expect(
      detectVariantFrom({
        fromBuild: null,
        fromHostname: "finance.worldmonitor.app",
        fromStore: "commodity",
      }),
    ).toBe("finance");
  });

  test("store wins when build + hostname are null", () => {
    expect(
      detectVariantFrom({
        fromBuild: null,
        fromHostname: null,
        fromStore: "commodity",
      }),
    ).toBe("commodity");
  });

  test("falls back to base when every source is null", () => {
    expect(detectVariantFrom(empty)).toBe("base");
  });

  test("invalid build value falls through to next source", () => {
    expect(
      detectVariantFrom({
        fromBuild: "BOGUS",
        fromHostname: "finance.worldmonitor.app",
        fromStore: null,
      }),
    ).toBe("finance");
  });

  test("invalid store value falls through to base", () => {
    expect(
      detectVariantFrom({
        fromBuild: null,
        fromHostname: null,
        fromStore: "BOGUS",
      }),
    ).toBe("base");
  });

  test("every variant id resolves through the build slot", () => {
    for (const v of ALL_VARIANTS) {
      const sources: DetectionSources = {
        fromBuild: v,
        fromHostname: null,
        fromStore: null,
      };
      expect(detectVariantFrom(sources)).toBe(v);
    }
  });
});
