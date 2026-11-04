/**
 * personal-cfo-4d8.39.2: a card's error, its remedy and its one action live in the section's
 * persistent slot, so a collapse cannot hide them, the draft survives, and the action is never
 * rendered twice. Household and Comfort band (slice b, part 1).
 */

import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

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
const band = (lower: number, upper: number | null): ComfortBandDto => ({
  currency: "USD",
  lower: usd(lower),
  upper: upper === null ? null : usd(upper),
});

beforeEach(() => {
  vi.clearAllMocks();
  mocks.householdTimezone.mockResolvedValue(ok("America/Los_Angeles"));
  mocks.setHouseholdTimezone.mockResolvedValue(ok(null));
  mocks.comfortBand.mockResolvedValue(ok(band(0, null)));
  mocks.setMinimumCashFloor.mockResolvedValue(ok(null));
  mocks.setComfortBandUpper.mockResolvedValue(ok(null));
});

const toggle = (title: string) => fireEvent.click(screen.getByRole("button", { name: title }));
const section = (title: string) =>
  document.querySelector(`[data-settings-section="${title}"]`) as HTMLElement;
/// Every button with this name in the section, whether or not it is currently exposed.
const everyButton = (title: string, name: string | RegExp) =>
  within(section(title)).queryAllByRole("button", { name, hidden: true });
const slotOf = (title: string) =>
  section(title).querySelector("[data-settings-persistent]") as HTMLElement;

describe("Household", () => {
  const tz = () => screen.getByRole("button", { name: "Household timezone" });
  const render = () =>
    renderWithClient(
      <SettingsSection title="Household" span="full">
        <HouseholdCard />
      </SettingsSection>,
    );
  async function pickNewYork() {
    await waitFor(() => expect(tz()).not.toBeDisabled());
    fireEvent.click(tz());
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "New York" } });
    fireEvent.pointerDown(screen.getByRole("option", { name: /New York/ }));
  }

  describe("a failed write", () => {
    beforeEach(() => {
      mocks.setHouseholdTimezone.mockResolvedValueOnce(refused("not a recognized timezone"));
    });

    it("stays visible collapsed, as the only Save in the section, with the draft kept", async () => {
      render();
      await pickNewYork();
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent("not a recognized timezone");
      expect(slotOf("Household").contains(alert)).toBe(true);
      expect(everyButton("Household", "Save")).toHaveLength(1);
      expect(slotOf("Household").contains(everyButton("Household", "Save")[0]!)).toBe(true);

      toggle("Household");
      expect(screen.getByRole("alert")).toBeVisible();
      expect(screen.getByRole("button", { name: "Save" })).toBeVisible();
      expect(everyButton("Household", "Save")).toHaveLength(1);
      toggle("Household");
      // The draft survived the collapse: the picker still holds New York.
      expect(tz()).toHaveTextContent("New York");
    });

    it("submits the same draft from the slot, and the Save returns to the body when it clears", async () => {
      render();
      await pickNewYork();
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
      await screen.findByRole("alert");
      toggle("Household");
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
      await waitFor(() => expect(mocks.setHouseholdTimezone).toHaveBeenCalledTimes(2));
      expect(mocks.setHouseholdTimezone).toHaveBeenLastCalledWith("America/New_York");
      await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
      toggle("Household");
      const saves = everyButton("Household", "Save");
      expect(saves).toHaveLength(1);
      expect(slotOf("Household").contains(saves[0]!)).toBe(false);
    });

    it("hands focus to the Save in the slot when the error moves it, and back to the body when it clears", async () => {
      render();
      await pickNewYork();
      const body = screen.getByRole("button", { name: "Save" });
      body.focus();
      fireEvent.click(body);
      await screen.findByRole("alert");
      const inSlot = within(slotOf("Household")).getByRole("button", { name: "Save" });
      await waitFor(() => expect(document.activeElement).toBe(inSlot));

      inSlot.focus();
      fireEvent.click(inSlot);
      await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
      const back = everyButton("Household", "Save")[0]!;
      expect(slotOf("Household").contains(back)).toBe(false);
      // Saved, so there is nothing left to save and the body's Save is disabled (a disabled button
      // takes no focus): focus lands on the timezone picker rather than being dropped.
      expect(back).toBeDisabled();
      await waitFor(() => expect(document.activeElement).toBe(tz()));
    });

    it("does not take focus from somewhere else when the Save moves", async () => {
      render();
      await pickNewYork();
      const body = screen.getByRole("button", { name: "Save" });
      fireEvent.click(body);
      await screen.findByRole("alert");
      const inSlot = within(slotOf("Household")).getByRole("button", { name: "Save" });
      // Once the Save is enabled again (a disabled button is where a hand-over would wait).
      await waitFor(() => expect(inSlot).toBeEnabled());
      expect(document.activeElement).not.toBe(inSlot);
    });

    it("keeps its name while busy", async () => {
      render();
      await pickNewYork();
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
      await screen.findByRole("alert");
      let finish: (value: unknown) => void = () => {};
      mocks.setHouseholdTimezone.mockReturnValue(new Promise((resolve) => (finish = resolve)));
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
      const busy = await screen.findByRole("button", { name: "Save" });
      await waitFor(() => expect(busy).toBeDisabled());
      await act(async () => finish(ok(null)));
    });
  });

  describe("a failed read", () => {
    beforeEach(() => {
      mocks.householdTimezone.mockResolvedValueOnce(refused("The vault is busy."));
    });

    it("shows its error and one Retry while collapsed, and a retry reads again without writing", async () => {
      render();
      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent("The vault is busy.");
      expect(slotOf("Household").contains(alert)).toBe(true);
      toggle("Household");
      expect(screen.getByRole("alert")).toBeVisible();
      expect(everyButton("Household", "Retry")).toHaveLength(1);

      fireEvent.click(screen.getByRole("button", { name: "Retry" }));
      await waitFor(() => expect(mocks.householdTimezone).toHaveBeenCalledTimes(2));
      await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
      expect(everyButton("Household", "Retry")).toHaveLength(0);
      // A read is never a write: nothing was saved, least of all the UTC fallback.
      expect(mocks.setHouseholdTimezone).not.toHaveBeenCalled();
    });

    it("says Retrying… and holds the button while the retry runs, and keeps the failure up", async () => {
      render();
      await screen.findByRole("alert");
      let finish: (value: unknown) => void = () => {};
      mocks.householdTimezone.mockReturnValueOnce(new Promise((resolve) => (finish = resolve)));
      fireEvent.click(screen.getByRole("button", { name: "Retry" }));
      expect(await screen.findByRole("button", { name: "Retrying…" })).toBeDisabled();
      expect(screen.getByRole("alert")).toHaveTextContent("The vault is busy.");
      await act(async () => finish(refused("The vault is busy.")));
      expect(await screen.findByRole("button", { name: "Retry" })).toBeEnabled();
      expect(mocks.setHouseholdTimezone).not.toHaveBeenCalled();
    });

    it("never saves the UTC fallback: Save stays disabled until a different zone is chosen", async () => {
      render();
      await screen.findByRole("alert");
      expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
      expect(mocks.setHouseholdTimezone).not.toHaveBeenCalled();
    });
  });
});

describe("Comfort band", () => {
  const LOWER_SAVED =
    "Saved the lower edge, but couldn't update the upper edge: That upper edge is not allowed.";
  const render = () =>
    renderWithClient(
      <SettingsSection title="Comfort band" span="twoOfThree">
        <ComfortBandCard />
      </SettingsSection>,
    );
  async function fill(lower = "500", upper = "900") {
    await waitFor(() => expect(screen.getByLabelText(/lower edge/i)).toBeEnabled());
    fireEvent.change(screen.getByLabelText(/lower edge/i), { target: { value: lower } });
    fireEvent.change(screen.getByLabelText(/upper edge/i), { target: { value: upper } });
  }

  it("lower saved, upper failed: the whole alert stays visible collapsed, both drafts survive, and there is one Save", async () => {
    mocks.setComfortBandUpper.mockResolvedValueOnce(refused("That upper edge is not allowed."));
    render();
    await fill();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(LOWER_SAVED);
    expect(slotOf("Comfort band").contains(alert)).toBe(true);
    expect(everyButton("Comfort band", "Save")).toHaveLength(1);

    toggle("Comfort band");
    expect(screen.getByRole("alert")).toBeVisible();
    expect(screen.getByRole("alert")).toHaveTextContent(LOWER_SAVED);
    expect(screen.getByRole("button", { name: "Save" })).toBeVisible();
    toggle("Comfort band");
    expect(screen.getByLabelText(/lower edge/i)).toHaveValue("500");
    expect(screen.getByLabelText(/upper edge/i)).toHaveValue("900");
  });

  it("the slot's Save runs the same two-setting sequence from the same drafts", async () => {
    mocks.setComfortBandUpper.mockResolvedValueOnce(refused("That upper edge is not allowed."));
    render();
    await fill();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByRole("alert");
    toggle("Comfort band");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mocks.setComfortBandUpper).toHaveBeenCalledTimes(2));
    expect(mocks.setMinimumCashFloor).toHaveBeenCalledTimes(2);
    expect(mocks.setMinimumCashFloor).toHaveBeenLastCalledWith(usd(50_000));
    expect(mocks.setComfortBandUpper).toHaveBeenLastCalledWith(usd(90_000));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    // Cleared: the Save is back in the body, the only one.
    toggle("Comfort band");
    const saves = everyButton("Comfort band", "Save");
    expect(saves).toHaveLength(1);
    expect(slotOf("Comfort band").contains(saves[0]!)).toBe(false);
  });

  it("hands focus to the Save in the slot when the error moves it, and back to the body when it clears", async () => {
    mocks.setComfortBandUpper.mockResolvedValueOnce(refused("That upper edge is not allowed."));
    render();
    await fill();
    const body = screen.getByRole("button", { name: "Save" });
    body.focus();
    fireEvent.click(body);
    await screen.findByRole("alert");
    const inSlot = within(slotOf("Comfort band")).getByRole("button", { name: "Save" });
    await waitFor(() => expect(document.activeElement).toBe(inSlot));

    inSlot.focus();
    fireEvent.click(inSlot);
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    const back = everyButton("Comfort band", "Save")[0]!;
    expect(slotOf("Comfort band").contains(back)).toBe(false);
    await waitFor(() => expect(document.activeElement).toBe(back));
  });

  it("does not take focus from somewhere else when the Save moves", async () => {
    mocks.setComfortBandUpper.mockResolvedValueOnce(refused("That upper edge is not allowed."));
    render();
    await fill();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByRole("alert");
    const inSlot = within(slotOf("Comfort band")).getByRole("button", { name: "Save" });
    await waitFor(() => expect(inSlot).toBeEnabled());
    expect(document.activeElement).not.toBe(inSlot);
  });

  it("a validation message is persistent too, with the one Save", async () => {
    render();
    await fill("500", "100");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("The upper edge must be at or above the lower edge.");
    toggle("Comfort band");
    expect(screen.getByRole("alert")).toBeVisible();
    expect(everyButton("Comfort band", "Save")).toHaveLength(1);
    expect(mocks.setMinimumCashFloor).not.toHaveBeenCalled();
  });

  it("a failed read shows its message while collapsed", async () => {
    mocks.comfortBand.mockResolvedValue(refused("The vault is busy."));
    render();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("The vault is busy.");
    toggle("Comfort band");
    expect(screen.getByRole("alert")).toBeVisible();
  });
});
