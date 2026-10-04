/**
 * Copy-review guard for the forecast / dashboard non-advice boundary (ADR 0018,
 * personal-cfo-9xq).
 *
 * DohFlow computes, it does not advise. This scans the forecast and
 * dashboard UI source for **prescriptive advice phrases** ("you should", "we
 * recommend", "should pay", "you can afford", …) and fails if any user-facing
 * copy crosses from describing the situation into telling the user what to do.
 *
 * It matches advice *phrases*, not the bare verbs from §21.2 (should / recommend /
 * suggest / must), so incidental, non-advice uses — e.g. a comment noting a value
 * "must match the currency" — don't trip it. Comment-only lines and block comments
 * are stripped before scanning so component doc-comments aren't false positives.
 * Human review stays the primary safeguard; this is the regression net.
 *
 * Source is pulled in via Vite's `import.meta.glob` raw-import (no `node:fs`), so
 * the scan needs no `@types/node` and runs in the jsdom test env. New financial
 * surfaces (e.g. agent reports) are added to the globbed directories below.
 */

import { registry } from "./settings/connections/fixtures/adapters";

const SOURCES: Record<string, string> = {
  ...(import.meta.glob("./future-cash/**/*.{ts,tsx}", {
    query: "?raw",
    eager: true,
    import: "default",
  }) as Record<string, string>),
  ...(import.meta.glob("./dashboard/**/*.{ts,tsx}", {
    query: "?raw",
    eager: true,
    import: "default",
  }) as Record<string, string>),
  // The Accounts shell now hosts debt/investment views + the debt-terms form (ADR 0037,
  // ADR 0018 addendum 915.1) — descriptive copy only, so it joins the scan.
  ...(import.meta.glob("./accounts/**/*.{ts,tsx}", {
    query: "?raw",
    eager: true,
    import: "default",
  }) as Record<string, string>),
  // Transactions hosts the spend-by-category breakdown (ADR 0052 §1). ADR 0052 §5 makes
  // extending this scan a PREREQUISITE of that surface, not a follow-up: spend copy is
  // exactly where "you're overspending on dining" wants to creep in, and a guard added
  // after the copy lands is a guard that never saw the copy it was written for.
  ...(import.meta.glob("./transactions/**/*.{ts,tsx}", {
    query: "?raw",
    eager: true,
    import: "default",
  }) as Record<string, string>),
  // Scenarios (personal-cfo-m1am). This surface had never been scanned, and it is the one
  // where advice is easiest to write by accident: a scenario is a hypothetical about the
  // reader's own finances, so copy that names WHY you might model something ("if you take
  // leave", "when you get a raise") sits one word away from telling them to do it. The
  // first-run starting points are the specific risk — they must read as examples of the
  // mechanism, never as suggestions.
  ...(import.meta.glob("./scenarios/**/*.{ts,tsx}", {
    query: "?raw",
    eager: true,
    import: "default",
  }) as Record<string, string>),
  // Settings + Money Inbox joined when connector copy landed (personal-cfo-ul5d
  // /-zfyo): connection-health and sync-outcome strings talk directly about the
  // reader's money flow, which is exactly where prescriptive phrasing creeps in.
  ...(import.meta.glob("./settings/**/*.{ts,tsx}", {
    query: "?raw",
    eager: true,
    import: "default",
  }) as Record<string, string>),
  ...(import.meta.glob("./money-inbox/**/*.{ts,tsx}", {
    query: "?raw",
    eager: true,
    import: "default",
  }) as Record<string, string>),
  // Onboarding (personal-cfo-kdw6): the connected-vs-manual fork and the
  // Bridge disclosures are first-contact copy about the reader's money.
  // Income + bills (gmnk): the detected-income / suggested-recurring surfaces
  // describe the reader's paychecks and obligations.
  ...(import.meta.glob("./income/**/*.{ts,tsx}", {
    query: "?raw",
    eager: true,
    import: "default",
  }) as Record<string, string>),
  ...(import.meta.glob("./bills/**/*.{ts,tsx}", {
    query: "?raw",
    eager: true,
    import: "default",
  }) as Record<string, string>),
  // Export guidance (rfsc / lu4tm): step-by-step copy about the reader's banks.
  ...(import.meta.glob("./imports/**/*.{ts,tsx}", {
    query: "?raw",
    eager: true,
    import: "default",
  }) as Record<string, string>),
  ...(import.meta.glob("./onboarding/**/*.{ts,tsx}", {
    query: "?raw",
    eager: true,
    import: "default",
  }) as Record<string, string>),
};

/// Forbidden advice phrases, matched case-insensitively against comment-stripped
/// source.
const FORBIDDEN: { pattern: RegExp; why: string }[] = [
  { pattern: /\byou should(n['’]?t)?\b/i, why: "advice: “you should”" },
  { pattern: /\byou must\b/i, why: "advice: “you must”" },
  { pattern: /\byou (ought to|need to)\b/i, why: "imperative advice" },
  { pattern: /\byou can afford\b/i, why: "affordability advice" },
  { pattern: /\bwe (recommend|suggest|advise)\b/i, why: "recommendation" },
  {
    pattern:
      /\b(should|recommend|suggest|consider)\s+(pay(ing)?|spend(ing)?|transfer(ring)?|invest(ing)?|sav(e|ing)|mov(e|ing)|buy(ing)?|sell(ing)?)\b/i,
    why: "prescriptive action",
  },
];

/// Strip block comments and comment-only / doc-comment lines so we scan copy, not
/// developer prose. Trailing inline comments are left in (they over-match toward
/// stricter, never looser).
function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .split("\n")
    .filter((line) => !/^\s*(\/\/\/?|\*)/.test(line))
    .join("\n");
}

/// Whether a globbed path is a scannable source file (not a test or declaration).
function isScannable(path: string): boolean {
  return !/\.test\.tsx?$/.test(path) && !/\.d\.ts$/.test(path);
}

describe("forecast/dashboard copy is descriptive, not prescriptive (ADR 0018)", () => {
  it("contains no advice phrases in user-facing copy", () => {
    const violations: string[] = [];
    for (const [path, raw] of Object.entries(SOURCES)) {
      if (!isScannable(path)) continue;
      const source = stripComments(raw);
      for (const { pattern, why } of FORBIDDEN) {
        const match = pattern.exec(source);
        if (match) {
          violations.push(`${path}: ${why} — found “${match[0]}”`);
        }
      }
    }
    expect(
      violations,
      "Prescriptive language is forbidden on forecast/dashboard surfaces " +
        "(ADR 0018). Rephrase descriptively (“Based on current assumptions…”):\n" +
        violations.join("\n"),
    ).toEqual([]);
  });

  it("actually scans some source (guards against an empty sweep)", () => {
    const scanned = Object.keys(SOURCES).filter(isScannable);
    expect(scanned.length).toBeGreaterThan(0);
  });
});

/// Connector registry copy (personal-cfo-pxi.5). Since the provider picker
/// (dto2j), each provider's disclosure points, link guide and referral
/// sentence live in the Rust registry, not in scanned source. The registry
/// fixture holds EVERY registered provider, disabled ones included, and is
/// pinned complete and current against the registry by
/// src-tauri/tests/connector_registry.rs — so this holds every provider's
/// user-facing copy to the same advice boundary as the source above.

/// Every user-facing string a registry entry supplies, labelled for errors.
function registryCopy(): { where: string; text: string }[] {
  return registry.flatMap((adapter) => {
    const at = (field: string) => `${adapter.adapter_id}.${field}`;
    const guide = adapter.link_guide;
    return [
      { where: at("display_name"), text: adapter.display_name },
      ...Object.entries(adapter.disclosure).map(([field, text]) => ({
        where: at(`disclosure.${field}`),
        text,
      })),
      { where: at("link_guide.title"), text: guide.title },
      { where: at("link_guide.refresh_note"), text: guide.refresh_note },
      ...guide.setup_steps.map((text, i) => ({ where: at(`link_guide.setup_steps[${i}]`), text })),
      { where: at("link_guide.credential_label"), text: guide.credential_label },
      { where: at("link_guide.credential_noun"), text: guide.credential_noun },
      { where: at("link_guide.credential_placeholder"), text: guide.credential_placeholder },
      { where: at("link_guide.paste_instructions"), text: guide.paste_instructions },
      { where: at("economics.history_depth_expectation"), text: adapter.economics.history_depth_expectation },
      ...(adapter.referral
        ? [{ where: at("referral.disclosure"), text: adapter.referral.disclosure }]
        : []),
    ];
  });
}

describe("connector registry copy is descriptive, not prescriptive (ADR 0018)", () => {
  it("contains no advice phrases in any registered provider's copy", () => {
    const violations: string[] = [];
    for (const { where, text } of registryCopy()) {
      for (const { pattern, why } of FORBIDDEN) {
        const match = pattern.exec(text);
        if (match) violations.push(`${where}: ${why} — found “${match[0]}”`);
      }
    }
    expect(
      violations,
      "Registry copy is user-facing (the provider picker) and follows ADR 0018. " +
        "Rephrase descriptively in the provider's ConnectorMetadata, then " +
        "regenerate the fixture:\n" +
        violations.join("\n"),
    ).toEqual([]);
  });

  it("scans every provider's copy, disabled providers included", () => {
    expect(registry.length).toBeGreaterThan(0);
    expect(registry.some((adapter) => !adapter.enabled)).toBe(true);
    const scannedProviders = new Set(
      registryCopy().map(({ where }) => where.split(".")[0]),
    );
    expect([...scannedProviders].sort()).toEqual(
      registry.map((adapter) => adapter.adapter_id).sort(),
    );
  });
});
