import { afterEach, describe, expect, it, vi } from "vitest";

import { postResultIfAsked } from "./measure";

/// The dev pages can hand a captured result to a local collector (`?post=...&name=...`), which is how the
/// iOS simulator's Safari gets its raw results out. It must go nowhere but a loopback address.
describe("postResultIfAsked (personal-cfo-4d8.35)", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    window.history.replaceState({}, "", "/");
  });

  const run = (query: string) => {
    const fetchMock = vi.fn(() => Promise.resolve(new Response("ok")));
    vi.stubGlobal("fetch", fetchMock);
    window.history.replaceState({}, "", `/${query}`);
    postResultIfAsked({ rows: [1] });
    return fetchMock;
  };

  it("sends nothing unless asked", () => {
    expect(run("").mock.calls).toHaveLength(0);
    expect(run("?post=http://127.0.0.1:5193/save").mock.calls).toHaveLength(0); // no name
    expect(run("?name=a.json").mock.calls).toHaveLength(0); // no target
  });

  it("posts the result to a loopback collector under the given name", () => {
    for (const host of ["127.0.0.1", "localhost"]) {
      const calls = run(`?post=http://${host}:5193/save&name=a.json`).mock.calls as unknown as [URL, RequestInit][];
      expect(calls).toHaveLength(1);
      expect(String(calls[0]![0])).toBe(`http://${host}:5193/save?name=a.json`);
      expect(calls[0]![1].method).toBe("POST");
      expect(calls[0]![1].body).toBe('{"rows":[1]}');
    }
  });

  it("never sends a result anywhere but a loopback http address", () => {
    for (const target of [
      "http://example.com/save",
      "https://127.0.0.1/save",
      "http://127.0.0.1.evil.example/save",
      "http://192.168.1.5:5193/save",
    ]) {
      expect(run(`?post=${encodeURIComponent(target)}&name=a.json`).mock.calls, target).toHaveLength(0);
    }
  });
});
