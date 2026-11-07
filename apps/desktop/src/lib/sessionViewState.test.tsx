import { act, fireEvent, render, screen } from "@testing-library/react";

import { SessionViewStateProvider } from "./SessionViewStateProvider";
import { useSessionState } from "./sessionViewState";

function Counter({ label = "c" }: { label?: string }) {
  const [n, setN] = useSessionState<number>("futureCash.horizon", 90);
  return (
    <button type="button" onClick={() => setN((v) => v + 1)}>
      {label}:{n}
    </button>
  );
}

describe("the session view state store (personal-cfo-i93n3.27 slice D)", () => {
  it("is useState: the initial value, a value setter and an updater", () => {
    render(
      <SessionViewStateProvider>
        <Counter />
      </SessionViewStateProvider>,
    );
    const button = screen.getByRole("button");
    expect(button).toHaveTextContent("c:90");
    fireEvent.click(button);
    fireEvent.click(button);
    expect(button).toHaveTextContent("c:92");
  });

  it("survives its view unmounting and mounting again inside one provider", () => {
    function Host({ show }: { show: boolean }) {
      return <SessionViewStateProvider>{show ? <Counter /> : <p>elsewhere</p>}</SessionViewStateProvider>;
    }
    const { rerender } = render(<Host show />);
    fireEvent.click(screen.getByRole("button"));
    rerender(<Host show={false} />);
    expect(screen.getByText("elsewhere")).toBeInTheDocument();
    rerender(<Host show />);
    expect(screen.getByRole("button")).toHaveTextContent("c:91");
  });

  it("shares a key between two views under one provider", () => {
    render(
      <SessionViewStateProvider>
        <Counter label="a" />
        <Counter label="b" />
      </SessionViewStateProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: /^a:/ }));
    expect(screen.getByRole("button", { name: /^b:/ })).toHaveTextContent("b:91");
  });

  it("starts empty under a new provider (what a lock or a vault switch is)", () => {
    const first = render(
      <SessionViewStateProvider>
        <Counter />
      </SessionViewStateProvider>,
    );
    fireEvent.click(screen.getByRole("button"));
    first.unmount();
    render(
      <SessionViewStateProvider>
        <Counter />
      </SessionViewStateProvider>,
    );
    expect(screen.getByRole("button")).toHaveTextContent("c:90");
  });

  it("is plain local state without a provider", () => {
    render(<Counter />);
    fireEvent.click(screen.getByRole("button"));
    expect(screen.getByRole("button")).toHaveTextContent("c:91");
  });

  it("is local state for a null key even under a provider", () => {
    function NullKey() {
      const [n, setN] = useSessionState<number>(null, 0);
      return (
        <button type="button" onClick={() => setN(n + 1)}>
          n:{n}
        </button>
      );
    }
    render(
      <SessionViewStateProvider>
        <NullKey />
        <Counter />
      </SessionViewStateProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: /^n:/ }));
    expect(screen.getByRole("button", { name: /^n:/ })).toHaveTextContent("n:1");
    expect(screen.getByRole("button", { name: /^c:/ })).toHaveTextContent("c:90");
  });

  it("writes to no browser storage at all", async () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem");
    const open = vi.fn();
    vi.stubGlobal("indexedDB", { open });
    render(
      <SessionViewStateProvider>
        <Counter />
      </SessionViewStateProvider>,
    );
    fireEvent.click(screen.getByRole("button"));
    await act(async () => undefined);
    expect(setItem).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
    expect(window.localStorage.length).toBe(0);
    expect(window.sessionStorage.length).toBe(0);
    setItem.mockRestore();
    vi.unstubAllGlobals();
  });
});
