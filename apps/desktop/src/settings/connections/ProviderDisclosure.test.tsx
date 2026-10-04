// ProviderDisclosure (personal-cfo-dto2j): the registry-fed disclosure panel.
// SimpleFIN's panel is pinned byte for byte to the onboarding panel it
// replaced; the referral slot appears only for a provider that carries one;
// and no URL is ever a link (ADR 0010, ADR 0076 §5).

import { render, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import {
  exampleflow,
  REFERRAL_URL,
  SHIPPED_SIMPLEFIN_PANEL,
  simplefin,
} from "./fixtures/adapters";
import { ProviderDisclosure } from "./ProviderDisclosure";

describe("ProviderDisclosure", () => {
  it("renders SimpleFIN's registry entry byte-identical to the shipped panel", () => {
    const { container } = render(<ProviderDisclosure adapter={simplefin} />);
    // Carried over from BridgeEducation (kdw6): captured from the shipped
    // component before it was generalized — never regenerated.
    expect(container.textContent).toBe(SHIPPED_SIMPLEFIN_PANEL);
  });

  it("shows the four registry points, lead sentences in bold", () => {
    const { getByText } = render(<ProviderDisclosure adapter={exampleflow} />);
    expect(getByText("About ExampleFlow")).toBeInTheDocument();
    expect(getByText("It costs money.").tagName).toBe("B");
    expect(getByText(/connects to your banks/)).toBeInTheDocument();
    expect(getByText("It is optional.")).toBeInTheDocument();
    expect(getByText(/refreshes on ExampleFlow's schedule/)).toBeInTheDocument();
  });

  it("renders the referral sentence beside the referral URL only when there is one", () => {
    const withReferral = render(<ProviderDisclosure adapter={exampleflow} />);
    const note = withReferral.getByRole("note", {
      name: "Referral disclosure for ExampleFlow",
    });
    expect(note).toHaveTextContent(REFERRAL_URL);
    expect(note).toHaveTextContent(/may earn a commission/);
    withReferral.unmount();

    const without = render(<ProviderDisclosure adapter={simplefin} />);
    expect(without.queryByRole("note")).toBeNull();
    expect(without.container.textContent).not.toMatch(/commission|referral/i);
  });

  it("shows the referral sentence at full contrast, right under its URL", () => {
    const { getByRole } = render(<ProviderDisclosure adapter={exampleflow} />);
    const note = getByRole("note", { name: "Referral disclosure for ExampleFlow" });
    const sentence = within(note).getByText(/may earn a commission/);
    // Clear and conspicuous (ADR 0076 §5): the panel's normal text, not muted.
    expect(sentence.className).not.toMatch(/text-muted-foreground/);
    expect(sentence.closest("[class*='text-muted-foreground']")).toBeNull();
    // Adjacent: the URL comes first, the sentence directly after it, both
    // inside the one note that describes itself with the sentence.
    const url = within(note).getByText(REFERRAL_URL);
    expect(url.compareDocumentPosition(sentence) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(note.getAttribute("aria-describedby")).toBe(sentence.id);
    // Narrow widths: the URL wraps instead of overflowing.
    expect(url.className).toMatch(/break-all/);
  });

  it("never renders a URL as a link", () => {
    for (const adapter of [simplefin, exampleflow]) {
      const { container, unmount } = render(<ProviderDisclosure adapter={adapter} />);
      expect(container.querySelectorAll("a")).toHaveLength(0);
      expect(container.querySelectorAll("[href]")).toHaveLength(0);
      // The provider and referral URLs are selectable text.
      const codes = [...container.querySelectorAll("code")].map((c) => c.textContent);
      expect(codes).toContain(adapter.link_guide.provider_url);
      if (adapter.referral) expect(codes).toContain(adapter.referral.url);
      unmount();
    }
  });
});
