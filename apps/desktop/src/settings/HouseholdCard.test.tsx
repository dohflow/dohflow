import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";
import { HouseholdCard } from "./HouseholdCard";

const mocks = vi.hoisted(() => ({
  householdTimezone: vi.fn(),
  setHouseholdTimezone: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function trigger() {
  return screen.getByRole("button", { name: "Household timezone" });
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.householdTimezone.mockResolvedValue(ok("America/Los_Angeles"));
  mocks.setHouseholdTimezone.mockResolvedValue(ok(null));
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("HouseholdCard", () => {
  it("shows the stored timezone once loaded", async () => {
    renderWithClient(<HouseholdCard />);
    await waitFor(() => expect(trigger()).toHaveTextContent("Los Angeles"));
  });

  it("saves a newly picked timezone", async () => {
    const chosen = "America/New_York";
    renderWithClient(<HouseholdCard />);
    await waitFor(() => expect(trigger()).not.toBeDisabled());
    fireEvent.click(trigger());
    fireEvent.change(screen.getByRole("combobox"), {
      target: { value: "New York" },
    });
    fireEvent.pointerDown(screen.getByRole("option", { name: /New York/ }));

    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));
    await waitFor(() =>
      expect(mocks.setHouseholdTimezone).toHaveBeenCalledWith(chosen),
    );
    expect(await screen.findByText(/saved/i)).toBeInTheDocument();
  });

  it("disables Save until the selection actually changes", async () => {
    renderWithClient(<HouseholdCard />);
    await waitFor(() => expect(trigger()).toHaveTextContent("Los Angeles"));
    expect(screen.getByRole("button", { name: /^save$/i })).toBeDisabled();
  });

  it("surfaces a backend error without clearing the draft", async () => {
    mocks.setHouseholdTimezone.mockResolvedValue({
      status: "error",
      error: { Validation: "not a recognized timezone" },
    });
    renderWithClient(<HouseholdCard />);
    await waitFor(() => expect(trigger()).not.toBeDisabled());
    fireEvent.click(trigger());
    fireEvent.change(screen.getByRole("combobox"), {
      target: { value: "New York" },
    });
    fireEvent.pointerDown(screen.getByRole("option", { name: /New York/ }));
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      /not a recognized timezone/i,
    );
    // The draft (New York) survives the failure so the user doesn't have to reselect it.
    expect(trigger()).toHaveTextContent("New York");
  });

  it("suggests the machine's zone with a one-click adopt when the stored zone is UTC", async () => {
    mocks.householdTimezone.mockResolvedValue(ok("UTC"));
    // Only intercept the exact no-argument call `machineTimezone()` makes — any other
    // caller (React/testing-library internals included) gets the real constructor, so
    // this doesn't collaterally break unrelated Intl.DateTimeFormat usage in the tree.
    const RealDateTimeFormat = Intl.DateTimeFormat;
    vi.spyOn(Intl, "DateTimeFormat").mockImplementation((...args) => {
      if (args.length === 0) {
        return {
          resolvedOptions: () => ({ timeZone: "America/Denver" }),
        } as unknown as Intl.DateTimeFormat;
      }
      return new RealDateTimeFormat(...args);
    });
    renderWithClient(<HouseholdCard />);
    await waitFor(() => expect(trigger()).toHaveTextContent("UTC"));
    expect(await screen.findByText(/Denver/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /use it/i }));
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));
    await waitFor(() =>
      expect(mocks.setHouseholdTimezone).toHaveBeenCalledWith("America/Denver"),
    );
  });

  it("does not suggest the machine's zone once a real zone is stored", async () => {
    renderWithClient(<HouseholdCard />);
    await waitFor(() => expect(trigger()).toHaveTextContent("Los Angeles"));
    expect(screen.queryByText(/this mac is set to/i)).not.toBeInTheDocument();
  });
});
