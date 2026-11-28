import { TOUCH_ID_REASON_CODES, touchIdReasonCopy } from "./touchIdCopy";

// The codes of `touch_id::Reason` in the desktop host (apps/desktop/src-tauri/src/touch_id/policy.rs).
// A code added there without copy here would silently read as the generic fallback.
const HOST_CODES = [
  "after_restart",
  "too_many_failed_prompts",
  "password_too_old",
  "markers_unreadable",
  "retired",
  "biometry_not_present",
  "biometry_not_enrolled",
  "biometry_locked_out",
  "biometry_unknown",
  "signed_build_needed",
  "store_unavailable",
  "store_refused",
  "not_enabled",
  "prompt_not_completed",
  "unlock_unavailable",
  "attempt_in_progress",
];

describe("Touch ID reason copy", () => {
  it("has its own sentence for every code the host can return", () => {
    for (const code of HOST_CODES) {
      expect(TOUCH_ID_REASON_CODES, code).toContain(code);
      expect(touchIdReasonCopy(code), code).not.toBe("Enter your master password.");
    }
    expect(TOUCH_ID_REASON_CODES.sort()).toEqual([...HOST_CODES].sort());
  });

  it("is plain US English: whole sentences, no codes, keys, paths or jargon", () => {
    for (const code of HOST_CODES) {
      const text = touchIdReasonCopy(code);
      expect(text, code).toMatch(/[.]$/);
      expect(text, code).not.toMatch(/_|\bdek\b|\bkey ?chain\b|\bLAError\b|\bsecure enclave\b/i);
      expect(text, code).not.toMatch(/\b(colour|unlocked? for you|authorise)/i);
    }
  });

  it("falls back to the password for nothing, null or a code it does not know", () => {
    for (const code of [undefined, null, "", "from_the_future"]) {
      expect(touchIdReasonCopy(code)).toBe("Enter your master password.");
    }
  });

  it("says a retired item needs setting up again, with no unsupported cause", () => {
    expect(touchIdReasonCopy("retired")).toBe(
      "Touch ID needs to be set up again for this vault. Enter your master password, then turn Touch ID on again in Settings.",
    );
  });
});

describe("the store_refused line", () => {
  // Studio's formal verdict on xuu: the exact words, pinned so a rewording is a review, not an edit.
  it("is Studio's approved sentence", () => {
    expect(touchIdReasonCopy("store_refused")).toBe(
      "DohFlow couldn't turn on Touch ID. It stays off for this vault. Use your master password.",
    );
  });
});
