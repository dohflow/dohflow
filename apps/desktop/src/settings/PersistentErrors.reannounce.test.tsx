/**
 * personal-cfo-4d8.39.2: an error kept on screen through a retry must still be announced again when
 * the retry fails the same way. The alert's text clears and comes back in the same element when an
 * attempt settles as a failure (the Debt payoff status pattern, personal-cfo-6wk.26), and does not
 * change while the retry is pending.
 */

import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";
import { watchAlert } from "@/test/alertChanges";

import type { ComfortBandDto, MoneyDto } from "@/bindings";
import { ComfortBandCard } from "./ComfortBandCard";
import { HouseholdCard } from "./HouseholdCard";
import { SettingsSection } from "./SettingsSection";

const mocks = vi.hoisted(() => ({
  householdTimezone: vi.fn(),
  setHouseholdTimezone: vi.fn(),
  comfortBand: vi.fn(),
  setMinimumCashFloor: vi.fn(),
  setComfortBandUpper: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const refused = (message: string) => ({ status: "error", error: { Validation: message } }) as const;
const usd = (minor: number): MoneyDto => ({ minor_units: minor, currency: "USD" });
const band = (): ComfortBandDto => ({ currency: "USD", lower: usd(0), upper: null });

beforeEach(() => {
  vi.clearAllMocks();
  mocks.householdTimezone.mockResolvedValue(ok("America/Los_Angeles"));
  mocks.comfortBand.mockResolvedValue(ok(band()));
  mocks.setMinimumCashFloor.mockResolvedValue(ok(null));
});

const slotOf = (title: string) =>
  document.querySelector(`[data-settings-section="${title}"] [data-settings-persistent]`) as HTMLElement;

describe("Household", () => {
  it("a repeat of the same write failure announces again, and a pending retry changes nothing", async () => {
    mocks.setHouseholdTimezone.mockResolvedValue(refused("not a recognized timezone"));
    renderWithClient(
      <SettingsSection title="Household" span="full">
        <HouseholdCard />
      </SettingsSection>,
    );
    const tz = () => screen.getByRole("button", { name: "Household timezone" });
    await waitFor(() => expect(tz()).not.toBeDisabled());
    fireEvent.click(tz());
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "New York" } });
    fireEvent.pointerDown(screen.getByRole("option", { name: /New York/ }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("not a recognized timezone");

    let release!: (value: unknown) => void;
    mocks.setHouseholdTimezone.mockReturnValueOnce(new Promise((resolve) => (release = resolve)));
    const watch = watchAlert(alert);
    fireEvent.click(within(slotOf("Household")).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mocks.setHouseholdTimezone).toHaveBeenCalledTimes(2));
    // Pending: the same message, still there, with no change to announce.
    expect(screen.getByRole("alert")).toBe(alert);
    expect(alert).toHaveTextContent("not a recognized timezone");
    expect(watch.stop()).toBe(0);

    const again = watchAlert(alert);
    await act(async () => release(refused("not a recognized timezone")));
    await waitFor(() => expect(alert).toHaveTextContent("not a recognized timezone"));
    // The retry failed the same way: the text cleared and came back in the same element.
    expect(screen.getByRole("alert")).toBe(alert);
    expect(again.stop()).toBeGreaterThanOrEqual(2);
  });
});

describe("Comfort band", () => {
  it("a repeat of the same upper-edge failure announces again", async () => {
    mocks.setComfortBandUpper.mockResolvedValue(refused("That upper edge is not allowed."));
    renderWithClient(
      <SettingsSection title="Comfort band" span="twoOfThree">
        <ComfortBandCard />
      </SettingsSection>,
    );
    await waitFor(() => expect(screen.getByLabelText(/lower edge/i)).toBeEnabled());
    fireEvent.change(screen.getByLabelText(/lower edge/i), { target: { value: "500" } });
    fireEvent.change(screen.getByLabelText(/upper edge/i), { target: { value: "900" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("couldn't update the upper edge");

    let release!: (value: unknown) => void;
    mocks.setComfortBandUpper.mockReturnValueOnce(new Promise((resolve) => (release = resolve)));
    const pending = watchAlert(alert);
    fireEvent.click(within(slotOf("Comfort band")).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mocks.setComfortBandUpper).toHaveBeenCalledTimes(2));
    expect(pending.stop()).toBe(0);

    const again = watchAlert(alert);
    await act(async () => release(refused("That upper edge is not allowed.")));
    await waitFor(() => expect(alert).toHaveTextContent("couldn't update the upper edge"));
    expect(screen.getByRole("alert")).toBe(alert);
    expect(again.stop()).toBeGreaterThanOrEqual(2);
  });
});
