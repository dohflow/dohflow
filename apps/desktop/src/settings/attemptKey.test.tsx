/**
 * `useReannounced` (personal-cfo-4d8.39.2): an alert kept through a retry blanks and comes back only
 * for an IDENTICAL repeat failure; a first or different message shows at once.
 */

import { act, renderHook } from "@testing-library/react";

import { useAttemptCount, useReannounced } from "./attemptKey";

/// Every value the hook returned, render by render.
function probe(initial: { text: string | null; attempt: number }) {
  const rendered: Array<string | null> = [];
  const hook = renderHook(
    (props: { text: string | null; attempt: number }) => {
      const value = useReannounced(props.text, props.attempt);
      rendered.push(value);
      return value;
    },
    { initialProps: initial },
  );
  return { rendered, ...hook };
}

describe("useReannounced", () => {
  it("shows a first failure at once, never blank", () => {
    const { rendered, rerender } = probe({ text: null, attempt: 0 });
    rerender({ text: "It failed.", attempt: 1 });
    expect(rendered).not.toContain("");
    expect(rendered.at(-1)).toBe("It failed.");
  });

  it("shows a different message at once, never blank", () => {
    const { rendered, rerender } = probe({ text: "It failed.", attempt: 1 });
    rerender({ text: "It failed another way.", attempt: 2 });
    expect(rendered).not.toContain("");
    expect(rendered.at(-1)).toBe("It failed another way.");
  });

  it("blanks for one render, then restores, on an identical repeat", () => {
    const { rendered, rerender } = probe({ text: "It failed.", attempt: 1 });
    rendered.length = 0;
    rerender({ text: "It failed.", attempt: 2 });
    expect(rendered[0]).toBe("");
    expect(rendered.at(-1)).toBe("It failed.");
  });

  it("does not move while the retry is pending (the attempt has not advanced)", () => {
    const { rendered, rerender } = probe({ text: "It failed.", attempt: 1 });
    rendered.length = 0;
    rerender({ text: "It failed.", attempt: 1 });
    rerender({ text: "It failed.", attempt: 1 });
    expect(rendered.every((value) => value === "It failed.")).toBe(true);
  });

  it("shows a failure again at once after the error had cleared", () => {
    const { rendered, rerender } = probe({ text: "It failed.", attempt: 1 });
    rerender({ text: null, attempt: 1 });
    rendered.length = 0;
    rerender({ text: "It failed.", attempt: 2 });
    expect(rendered).not.toContain("");
  });
});

describe("useAttemptCount", () => {
  it("counts the attempts that settled as a failure", () => {
    const { result } = renderHook(() => useAttemptCount());
    expect(result.current[0]).toBe(0);
    act(() => result.current[1]());
    act(() => result.current[1]());
    expect(result.current[0]).toBe(2);
  });
});
