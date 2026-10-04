// Registry fixtures for the provider-picker tests (personal-cfo-dto2j).
//
// `registry` is the REAL registry — every registered provider, disabled ones
// included. registry.json is written by the Rust registry and pinned to it
// (complete and current) by apps/desktop/src-tauri/tests/connector_registry.rs,
// so these tests render exactly what the app ships, and copy-review.test.ts
// holds every provider's copy to the advice boundary (personal-cfo-pxi.5).
// The other two are synthetic, shaped like any registry entry, to exercise a
// second enabled provider (with a referral) and a disabled one.

import type { ConnectorAdapterDto } from "@/bindings";

import registryJson from "./registry.json";
import shippedPanel from "./simplefin-panel.shipped.txt?raw";

/// Every registered provider, in the registry's order.
export const registry = registryJson as ConnectorAdapterDto[];

function registered(adapterId: string): ConnectorAdapterDto {
  const adapter = registry.find((entry) => entry.adapter_id === adapterId);
  if (!adapter) throw new Error(`${adapterId} is not in the registry fixture`);
  return adapter;
}

export const simplefin = registered("simplefin");

/// The onboarding panel's rendered text as shipped (personal-cfo-kdw6), frozen
/// before the panel was generalized.
export const SHIPPED_SIMPLEFIN_PANEL: string = shippedPanel;

export const REFERRAL_URL = "https://referral.example/?ref=dohflow";

/// A second enabled provider that carries a referral.
export const exampleflow: ConnectorAdapterDto = {
  ...simplefin,
  adapter_id: "exampleflow",
  display_name: "ExampleFlow",
  capabilities: { ...simplefin.capabilities, holdings: true },
  regions: ["GB", "US"],
  economics: {
    ...simplefin.economics,
    base_cost_minor_units: 3499,
    included_connections: 2,
    extra_connection_cost_minor_units: 1000,
    extra_connection_period: "Annual",
    terms_url: "https://exampleflow.example/terms",
  },
  disclosure: {
    independent_party:
      "ExampleFlow is a separate company. DohFlow has a referral relationship with it, explained in the note below.",
    handles_credentials:
      "ExampleFlow connects to your banks and gives this app read-only data through an API key you create.",
    cost_summary: "It costs money. ExampleFlow charges its own plan fee, paid to them.",
    optional: "It is optional. Manual entry and file imports work without it.",
  },
  link_guide: {
    title: "About ExampleFlow",
    refresh_note: "Data refreshes on ExampleFlow's schedule. This app refreshes on open and on demand.",
    setup_steps: [
      "Create an account at https://exampleflow.example and connect each bank there.",
      "Copy your API key and paste it below.",
    ],
    provider_url: "https://exampleflow.example",
    credential_label: "API key",
    credential_noun: "API key",
    credential_placeholder: "Paste the API key",
    paste_instructions: "Copy the API key from your ExampleFlow dashboard, then paste it here.",
  },
  referral: {
    url: REFERRAL_URL,
    disclosure:
      "DohFlow may earn a commission if you sign up for ExampleFlow through the link above.",
  },
  enabled: true,
};

/// A registered provider that is not switched on.
export const dormant: ConnectorAdapterDto = {
  ...exampleflow,
  adapter_id: "dormant",
  display_name: "Dormant Provider",
  enabled: false,
};
