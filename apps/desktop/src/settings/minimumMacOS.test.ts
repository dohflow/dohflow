import { describe, expect, it } from "vitest";
import { compareMacOSVersions, minimumMacOSBlock, parseMacOSVersion } from "./minimumMacOS";

describe("parseMacOSVersion", () => {
  it("reads one to three numeric parts", () => {
    expect(parseMacOSVersion("15")).toEqual([15]);
    expect(parseMacOSVersion("14.8")).toEqual([14, 8]);
    expect(parseMacOSVersion(" 14.8.9 ")).toEqual([14, 8, 9]);
  });

  it("rejects anything else", () => {
    for (const bad of ["", "fifteen", "15.0.0.1", "15.", "v15", "15.x", null, undefined, 15]) {
      expect(parseMacOSVersion(bad)).toBeNull();
    }
  });
});

describe("compareMacOSVersions", () => {
  it("orders by each part, treating missing parts as zero", () => {
    expect(compareMacOSVersions([15], [15, 0])).toBe(0);
    expect(compareMacOSVersions([15, 0], [14, 8, 9])).toBeGreaterThan(0);
    expect(compareMacOSVersions([14, 8, 9], [14, 10])).toBeLessThan(0);
    expect(compareMacOSVersions([26], [15, 7, 9])).toBeGreaterThan(0);
  });
});

describe("minimumMacOSBlock", () => {
  it("holds back an update whose floor is newer than this Mac", () => {
    expect(minimumMacOSBlock({ minimumSystemVersion: "15.0" }, "14.8.9")).toEqual({
      required: "15.0",
      current: "14.8.9",
    });
  });

  it("offers an update whose floor equals this Mac", () => {
    expect(minimumMacOSBlock({ minimumSystemVersion: "15.0" }, "15.0")).toBeNull();
    expect(minimumMacOSBlock({ minimumSystemVersion: "15" }, "15.0.0")).toBeNull();
  });

  it("offers an update whose floor is older than this Mac", () => {
    expect(minimumMacOSBlock({ minimumSystemVersion: "14.0" }, "27.0.1")).toBeNull();
  });

  it("offers an update from a manifest without a floor (older releases)", () => {
    expect(minimumMacOSBlock({ version: "0.3.0" }, "14.8.9")).toBeNull();
    expect(minimumMacOSBlock({}, "14.8.9")).toBeNull();
    expect(minimumMacOSBlock(null, "14.8.9")).toBeNull();
  });

  it("holds back an update whose floor can't be read", () => {
    expect(minimumMacOSBlock({ minimumSystemVersion: "fifteen" }, "27.0")).toEqual({
      required: "fifteen",
      current: "27.0",
    });
    expect(minimumMacOSBlock({ minimumSystemVersion: 15 }, "27.0")).toEqual({
      required: "15",
      current: "27.0",
    });
  });

  it("offers the update when this Mac's version is unknown", () => {
    expect(minimumMacOSBlock({ minimumSystemVersion: "15.0" }, null)).toBeNull();
    expect(minimumMacOSBlock({ minimumSystemVersion: "15.0" }, "garbage")).toBeNull();
  });
});
