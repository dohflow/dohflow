/**
 * personal-cfo-4d8.39.2 (slice b, part 3): General keeps a failed read or save, and the one control
 * that recovers from it (the base-currency select), in the section's persistent slot. "Saved." is
 * General's own status and does not touch any other section's error.
 */

import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { GeneralCard } from "./GeneralCard";
import { HouseholdCard } from "./HouseholdCard";
import { SettingsSection } from "./SettingsSection";

const mocks = vi.hoisted(() => ({
  baseCurrency: vi.fn(),
  setBaseCurrency: vi.fn(),
  householdTimezone: vi.fn(),
  setHouseholdTimezone: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));
vi.mock("@/vault/useVault", () => ({
  describeIpcError: (error: unknown) => (typeof error === "string" ? error : "Something went wrong."),
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.setBaseCurrency.mockResolvedValue(ok(null));
});

const renderCard = () =>
  renderWithClient(
    <SettingsSection title="General" span="full">
      <GeneralCard />
    </SettingsSection>,
  );
const section = (title = "General") =>
  document.querySelector(`[data-settings-section="${title}"]`) as HTMLElement;
const slot = (title = "General") =>
  section(title).querySelector("[data-settings-persistent]") as HTMLElement;
const selects = () => within(section()).queryAllByLabelText("Base currency", { selector: "select" });
const collapse = () => fireEvent.click(screen.getByRole("button", { name: "General" }));

describe("General", () => {
  it("a failed save keeps its message and its one select in the slot while collapsed", async () => {
    mocks.setBaseCurrency.mockResolvedValueOnce({ status: "error", error: "Persistence failure" });
    renderCard();
    fireEvent.change(await screen.findByLabelText("Base currency"), { target: { value: "EUR" } });
    const alert = await screen.findByText("Persistence failure");
    expect(slot().contains(alert)).toBe(true);
    expect(selects()).toHaveLength(1);
    expect(slot().contains(selects()[0]!)).toBe(true);
    // The failed choice is not shown as saved.
    expect(selects()[0]).toHaveValue("USD");

    collapse();
    expect(screen.getByText("Persistence failure")).toBeVisible();
    expect(screen.getByLabelText("Base currency")).toBeVisible();
    expect(mocks.setBaseCurrency).toHaveBeenCalledTimes(1);

    fireEvent.change(screen.getByLabelText("Base currency"), { target: { value: "EUR" } });
    await waitFor(() => expect(mocks.setBaseCurrency).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByText("Persistence failure")).not.toBeInTheDocument());
    // Recovered: the one select is back in the body.
    expect(selects()).toHaveLength(1);
    expect(slot().contains(selects()[0]!)).toBe(false);
  });

  it("a failed read keeps its message and the select in the slot", async () => {
    mocks.baseCurrency.mockResolvedValue({ status: "error", error: "Persistence failure" });
    renderCard();
    const alert = await screen.findByText("Persistence failure");
    expect(slot().contains(alert)).toBe(true);
    expect(selects()).toHaveLength(1);
    expect(slot().contains(selects()[0]!)).toBe(true);
  });

  it("focus stays on the select when a failed save moves it into the slot", async () => {
    mocks.setBaseCurrency.mockResolvedValueOnce({ status: "error", error: "Persistence failure" });
    renderCard();
    const select = await screen.findByLabelText("Base currency");
    select.focus();
    fireEvent.change(select, { target: { value: "EUR" } });
    await screen.findByText("Persistence failure");
    await waitFor(() => expect(document.activeElement).toBe(screen.getByLabelText("Base currency")));
    expect(slot().contains(document.activeElement)).toBe(true);
  });

  it("a successful save shows Saved. in the body and leaves the slot empty", async () => {
    renderCard();
    fireEvent.change(await screen.findByLabelText("Base currency"), { target: { value: "EUR" } });
    const saved = await screen.findByText("Saved.");
    expect(slot().contains(saved)).toBe(false);
    expect(slot()).toBeEmptyDOMElement();
  });

  it("Saved. does not erase another section's failure (the Household timezone read)", async () => {
    mocks.householdTimezone.mockResolvedValue({ status: "error", error: "Persistence failure" });
    renderWithClient(
      <>
        <SettingsSection title="Household" span="full">
          <HouseholdCard />
        </SettingsSection>
        <SettingsSection title="General" span="full">
          <GeneralCard />
        </SettingsSection>
      </>,
    );
    const failure = await waitFor(() => {
      const text = slot("Household").textContent ?? "";
      expect(text).not.toBe("");
      return text;
    });
    fireEvent.change(await screen.findByLabelText("Base currency"), { target: { value: "EUR" } });
    await screen.findByText("Saved.");
    expect(slot("Household").textContent).toBe(failure);
  });
});
