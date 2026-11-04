/**
 * personal-cfo-4d8.39.2, Studio R1: a recovery run from a COLLAPSED section must not drop focus to
 * the page. The card's own targets (the picker, a body Save) are hidden and inert then; focus goes to
 * the section's visible heading trigger instead, without opening the section. And the error and the
 * slot's action stay on screen while the retry is pending, so the control being used does not vanish
 * under the person's hands.
 *
 * jsdom does not enforce `inert`, and the app has no browser test runner (docs/agent/FRONTEND.md
 * section 8; visual regression is personal-cfo-nbqd). So these assert the CHOSEN target: collapse,
 * resolve, then `document.activeElement` is the trigger, never a hidden fallback.
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

const trigger = (title: string) => screen.getByRole("button", { name: title });
const section = (title: string) =>
  document.querySelector(`[data-settings-section="${title}"]`) as HTMLElement;
const slotOf = (title: string) =>
  section(title).querySelector("[data-settings-persistent]") as HTMLElement;

/// A promise the test settles by hand.
function held<T>() {
  let release!: (value: T) => void;
  const promise = new Promise<T>((resolve) => (release = resolve));
  return { promise, release };
}

describe("Household, resolved while collapsed", () => {
  const render = () =>
    renderWithClient(
      <SettingsSection title="Household" span="full">
        <HouseholdCard />
      </SettingsSection>,
    );
  async function failOnce() {
    mocks.setHouseholdTimezone.mockResolvedValueOnce(refused("not a recognized timezone"));
    render();
    const tz = () => screen.getByRole("button", { name: "Household timezone" });
    await waitFor(() => expect(tz()).not.toBeDisabled());
    fireEvent.click(tz());
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "New York" } });
    fireEvent.pointerDown(screen.getByRole("option", { name: /New York/ }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByRole("alert");
  }

  it("a successful Save from the slot hands focus to the section's heading, not the hidden picker", async () => {
    await failOnce();
    fireEvent.click(trigger("Household")); // collapse
    expect(trigger("Household")).toHaveAttribute("aria-expanded", "false");
    const retry = held<ReturnType<typeof ok>>();
    mocks.setHouseholdTimezone.mockReturnValueOnce(retry.promise);
    const save = within(slotOf("Household")).getByRole("button", { name: "Save" });
    save.focus();
    fireEvent.click(save);
    // Pending: the error and the slot's Save are still on screen, not moved into the hidden body.
    await waitFor(() => expect(mocks.setHouseholdTimezone).toHaveBeenCalledTimes(2));
    expect(within(slotOf("Household")).getByRole("alert")).toHaveTextContent("not a recognized timezone");
    expect(within(slotOf("Household")).getByRole("button", { name: "Save" })).toBeInTheDocument();
    await act(async () => retry.release(ok(null)));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(trigger("Household")));
    // The section was not opened by the hand-over.
    expect(trigger("Household")).toHaveAttribute("aria-expanded", "false");
  });

  it("a Save that fails again keeps focus on the Save in the slot", async () => {
    await failOnce();
    fireEvent.click(trigger("Household"));
    mocks.setHouseholdTimezone.mockResolvedValueOnce(refused("still not recognized"));
    const save = within(slotOf("Household")).getByRole("button", { name: "Save" });
    save.focus();
    fireEvent.click(save);
    await screen.findByText("still not recognized");
    await waitFor(() =>
      expect(document.activeElement).toBe(within(slotOf("Household")).getByRole("button", { name: "Save" })),
    );
  });

  it("does not take focus from a control the person moved to", async () => {
    await failOnce();
    fireEvent.click(trigger("Household"));
    const retry = held<ReturnType<typeof ok>>();
    mocks.setHouseholdTimezone.mockReturnValueOnce(retry.promise);
    const save = within(slotOf("Household")).getByRole("button", { name: "Save" });
    save.focus();
    fireEvent.click(save);
    await waitFor(() => expect(mocks.setHouseholdTimezone).toHaveBeenCalledTimes(2));
    act(() => trigger("Household").focus());
    await act(async () => retry.release(ok(null)));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(document.activeElement).toBe(trigger("Household"));
  });

  it("an open section keeps today's target: the picker after a successful Save", async () => {
    await failOnce();
    const save = within(slotOf("Household")).getByRole("button", { name: "Save" });
    save.focus();
    mocks.setHouseholdTimezone.mockResolvedValueOnce(ok(null));
    fireEvent.click(save);
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    await waitFor(() =>
      expect(document.activeElement).toBe(document.getElementById("household-timezone")),
    );
  });
});

describe("Comfort band, resolved while collapsed", () => {
  const render = () =>
    renderWithClient(
      <SettingsSection title="Comfort band" span="twoOfThree">
        <ComfortBandCard />
      </SettingsSection>,
    );
  async function failUpper() {
    mocks.setComfortBandUpper.mockResolvedValueOnce(refused("That upper edge is not allowed."));
    render();
    await waitFor(() => expect(screen.getByLabelText(/lower edge/i)).toBeEnabled());
    fireEvent.change(screen.getByLabelText(/lower edge/i), { target: { value: "500" } });
    fireEvent.change(screen.getByLabelText(/upper edge/i), { target: { value: "900" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByRole("alert");
  }

  it("a successful Save from the slot hands focus to the section's heading, and the slot's Save stays until it settles", async () => {
    await failUpper();
    fireEvent.click(trigger("Comfort band"));
    const retry = held<ReturnType<typeof ok>>();
    mocks.setComfortBandUpper.mockReturnValueOnce(retry.promise);
    const save = within(slotOf("Comfort band")).getByRole("button", { name: "Save" });
    save.focus();
    fireEvent.click(save);
    await waitFor(() => expect(mocks.setComfortBandUpper).toHaveBeenCalledTimes(2));
    // Pending: the whole error and the slot's Save (now busy) are still there.
    expect(within(slotOf("Comfort band")).getByRole("alert")).toHaveTextContent(
      "Saved the lower edge, but couldn't update the upper edge",
    );
    expect(within(slotOf("Comfort band")).getByRole("button", { name: "Save" })).toBeInTheDocument();
    await act(async () => retry.release(ok(null)));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(trigger("Comfort band")));
    expect(trigger("Comfort band")).toHaveAttribute("aria-expanded", "false");
  });

  it("an open section keeps today's target: the Save in the body", async () => {
    await failUpper();
    const save = within(slotOf("Comfort band")).getByRole("button", { name: "Save" });
    save.focus();
    fireEvent.click(save);
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Save" })),
    );
    expect(slotOf("Comfort band").contains(document.activeElement)).toBe(false);
  });

  it("after a Save that worked and left focus on the Save, a click onto the page is not undone by a later render", async () => {
    mocks.comfortBand.mockResolvedValue(ok(band(0, null)));
    render();
    await waitFor(() => expect(screen.getByLabelText(/lower edge/i)).toBeEnabled());
    fireEvent.change(screen.getByLabelText(/lower edge/i), { target: { value: "500" } });
    const save = screen.getByRole("button", { name: "Save" });
    save.focus();
    fireEvent.click(save);
    await screen.findByText("Saved.");
    // Settled, and the Save never moved: the person clicks a non-focusable area (focus goes to the
    // page), then something else makes the card render.
    act(() => (document.activeElement as HTMLElement).blur());
    expect(document.activeElement).toBe(document.body);
    fireEvent.change(screen.getByLabelText(/upper edge/i), { target: { value: "900" } });
    expect(document.activeElement).toBe(document.body);
  });
});
