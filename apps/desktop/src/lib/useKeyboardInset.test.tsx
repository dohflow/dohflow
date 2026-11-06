import { render, renderHook } from "@testing-library/react";

import { COARSE_POINTER_QUERY } from "./compact";
import { useKeyboardInset } from "./useKeyboardInset";

/// A visual viewport whose height and offset a test can set and announce.
function installVisualViewport(initial: { height: number; offsetTop: number; scale?: number }) {
  const target = new EventTarget();
  const viewport = Object.assign(target, { scale: 1, ...initial });
  const add = vi.spyOn(target, "addEventListener");
  Object.defineProperty(window, "visualViewport", { value: viewport, configurable: true });
  return {
    add,
    move(next: { height: number; offsetTop?: number; scale?: number }) {
      Object.assign(viewport, next);
      target.dispatchEvent(new Event("resize"));
    },
  };
}

function installPointer(coarse: boolean) {
  vi.stubGlobal("matchMedia", ((query: string) => ({
    matches: query === COARSE_POINTER_QUERY ? coarse : false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia);
}

const inset = () => document.documentElement.style.getPropertyValue("--keyboard-inset");

beforeEach(() => {
  vi.stubGlobal("innerHeight", 800);
});

afterEach(() => {
  vi.unstubAllGlobals();
  Reflect.deleteProperty(window, "visualViewport");
  document.documentElement.style.removeProperty("--keyboard-inset");
});

describe("useKeyboardInset (personal-cfo-i93n3.25)", () => {
  it("does nothing on a desktop window: no listener, no property", () => {
    installPointer(false);
    const viewport = installVisualViewport({ height: 500, offsetTop: 0 });

    renderHook(() => useKeyboardInset());

    expect(viewport.add).not.toHaveBeenCalled();
    expect(inset()).toBe("");
  });

  it("does nothing without a visual viewport", () => {
    installPointer(true);
    expect(() => renderHook(() => useKeyboardInset())).not.toThrow();
    expect(inset()).toBe("");
  });

  it("publishes the part of the layout viewport the visual viewport no longer covers", () => {
    installPointer(true);
    const viewport = installVisualViewport({ height: 800, offsetTop: 0 });
    renderHook(() => useKeyboardInset());
    expect(inset()).toBe("0px");

    viewport.move({ height: 500 });
    expect(inset()).toBe("300px");

    // Scrolling the page pans the visual viewport inside the layout viewport: the keyboard is no
    // shorter for it (the inset must not shrink as the person scrolls).
    viewport.move({ height: 500, offsetTop: 284 });
    expect(inset()).toBe("300px");

    viewport.move({ height: 800, offsetTop: 0 });
    expect(inset()).toBe("0px");
  });

  it("keeps the last inset while the page is pinch-zoomed, which also shortens the visual viewport", () => {
    installPointer(true);
    const viewport = installVisualViewport({ height: 500, offsetTop: 0 });
    renderHook(() => useKeyboardInset());
    expect(inset()).toBe("300px");

    viewport.move({ height: 250, scale: 2 });
    expect(inset()).toBe("300px");

    viewport.move({ height: 500, scale: 1 });
    expect(inset()).toBe("300px");
  });

  it("scrolls the focused field into view while the keyboard is up", () => {
    installPointer(true);
    const viewport = installVisualViewport({ height: 800, offsetTop: 0 });
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    const { getByLabelText } = render(
      <>
        <label>
          Amount <input />
        </label>
        <button type="button">Save</button>
      </>,
    );
    renderHook(() => useKeyboardInset());

    getByLabelText("Amount").focus();
    viewport.move({ height: 500 });
    expect(scrollIntoView).toHaveBeenCalledTimes(1);

    // A button is not a field: nothing to keep in view.
    scrollIntoView.mockClear();
    document.querySelector("button")?.focus();
    viewport.move({ height: 480 });
    expect(scrollIntoView).not.toHaveBeenCalled();
  });

  it("undoes the browser's own page scroll once the keyboard is gone, and not before", () => {
    installPointer(true);
    const viewport = installVisualViewport({ height: 800, offsetTop: 0 });
    const scrollTo = vi.spyOn(window, "scrollTo").mockImplementation(() => {});
    vi.stubGlobal("scrollY", 60);
    renderHook(() => useKeyboardInset());

    viewport.move({ height: 500 });
    expect(scrollTo).not.toHaveBeenCalled();

    viewport.move({ height: 800 });
    expect(scrollTo).toHaveBeenCalledWith(0, 0);
  });

  it("reports no keyboard while the visual viewport is panned past the top, as a zoom bounce does", () => {
    installPointer(true);
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    const viewport = installVisualViewport({ height: 800, offsetTop: 0 });
    const { getByLabelText } = render(
      <label>
        Amount <input />
      </label>,
    );
    renderHook(() => useKeyboardInset());
    getByLabelText("Amount").focus();

    viewport.move({ height: 800, offsetTop: -100 });
    viewport.move({ height: 800, offsetTop: -139 });

    expect(inset()).toBe("0px");
    expect(scrollIntoView).not.toHaveBeenCalled();
  });

  it("scrolls the newly focused field into view when focus moves while the keyboard is up", async () => {
    installPointer(true);
    const viewport = installVisualViewport({ height: 800, offsetTop: 0 });
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    const { getByLabelText } = render(
      <>
        <label>
          First <input />
        </label>
        <label>
          Second <input />
        </label>
      </>,
    );
    renderHook(() => useKeyboardInset());
    getByLabelText("First").focus();
    viewport.move({ height: 500 });
    scrollIntoView.mockClear();

    getByLabelText("Second").focus();
    await new Promise((resolve) => window.requestAnimationFrame(() => resolve(null)));

    expect(scrollIntoView).toHaveBeenCalled();
  });

  it("removes its listeners and the property on unmount", () => {
    installPointer(true);
    const viewport = installVisualViewport({ height: 500, offsetTop: 0 });
    const { unmount } = renderHook(() => useKeyboardInset());
    expect(inset()).toBe("300px");

    unmount();

    expect(inset()).toBe("");
    const before = viewport.add.mock.calls.length;
    viewport.move({ height: 400 });
    expect(inset()).toBe("");
    expect(viewport.add.mock.calls.length).toBe(before);
  });
});
