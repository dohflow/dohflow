import type { BackupHistoryEntryDto } from "@/bindings";

import {
  aboutSummary,
  autoCategorizeSummary,
  backupsSummary,
  comfortSummary,
  connectionsSummary,
} from "./settingsSummaries";

const entry = (verified: boolean): BackupHistoryEntryDto => ({
  backup_id: "b",
  vault_id: "v",
  created_at: "2026-11-01T00:00:00Z",
  kind: "scheduled",
  destination: "/b",
  format_version: 1,
  size_bytes: 1,
  verified,
  error: null,
});
const usd = (minor: number) => ({ minor_units: minor, currency: "USD" });

describe("collapsed section summaries are real data or nothing (personal-cfo-4d8.39.1)", () => {
  it("backups: the schedule and a verified LATEST backup, never an older one, never while unread", () => {
    expect(backupsSummary("weekly", [entry(true)])).toBe("Weekly · last backup verified");
    expect(backupsSummary("daily", [entry(true)])).toBe("Daily · last backup verified");
    expect(backupsSummary("monthly", [entry(true), entry(false)])).toBe("Monthly · last backup verified");
    // The most recent backup is the one that counts.
    expect(backupsSummary("weekly", [entry(false), entry(true)])).toBeUndefined();
    expect(backupsSummary("weekly", [])).toBeUndefined();
    expect(backupsSummary("weekly", undefined)).toBeUndefined();
    expect(backupsSummary(undefined, [entry(true)])).toBeUndefined();
    expect(backupsSummary("off", [entry(true)])).toBeUndefined();
  });

  it("comfort band: both edges, formatted, or nothing", () => {
    expect(comfortSummary({ currency: "USD", lower: usd(500_000), upper: usd(1_500_000) })).toBe(
      "$5,000.00–$15,000.00",
    );
    expect(comfortSummary({ currency: "USD", lower: usd(500_000), upper: null })).toBeUndefined();
    expect(comfortSummary(null)).toBeUndefined();
  });

  it("auto-categorize: On only when loaded and enabled", () => {
    expect(autoCategorizeSummary(true)).toBe("On");
    expect(autoCategorizeSummary(false)).toBeUndefined();
    expect(autoCategorizeSummary(undefined)).toBeUndefined();
  });

  it("connections: a count from two, none for one or unread", () => {
    expect(connectionsSummary(3)).toBe("3 connections");
    expect(connectionsSummary(2)).toBe("2 connections");
    expect(connectionsSummary(1)).toBeUndefined();
    expect(connectionsSummary(0)).toBeUndefined();
    expect(connectionsSummary(undefined)).toBeUndefined();
  });

  it("about: the build version, or nothing", () => {
    expect(aboutSummary("0.3.0")).toBe("DohFlow · version 0.3.0");
    expect(aboutSummary(undefined)).toBeUndefined();
    expect(aboutSummary("")).toBeUndefined();
  });
});
