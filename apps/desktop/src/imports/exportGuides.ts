// How to get a transaction export out of common institutions
// (personal-cfo-rfsc, absorbed by the manual-path walkthrough lu4tm; ADR 0060
// §3): the manual ritual documented, not tribal knowledge. Static content that
// ships in the binary — no network, no deep links that rot. Bank menus move,
// so every entry names the landmarks (Statements, Activity, Download/Export)
// rather than promising exact clicks, and the generic entry covers the rest.

export type ExportFormat = "csv" | "ofx" | "qfx";

export interface ExportGuide {
  /// Stable id (a slug), used as the select value.
  id: string;
  name: string;
  /// Formats the institution typically offers, best first.
  formats: ExportFormat[];
  /// Landmark-based steps; each a short imperative sentence.
  steps: string[];
  /// A caveat worth knowing before the first import.
  note?: string;
}

export const FORMAT_LABEL: Record<ExportFormat, string> = {
  csv: "CSV",
  ofx: "OFX",
  qfx: "QFX (Quicken)",
};

export const EXPORT_GUIDES: ExportGuide[] = [
  {
    id: "chase",
    name: "Chase",
    formats: ["csv", "qfx"],
    steps: [
      "Open the account, then choose Download account activity (near the activity list).",
      "Pick the account and a date range, then choose a file type.",
      "Download the file; import the CSV or QFX here.",
    ],
    note: "Structured downloads usually reach back a year or two (and may be capped by row count); older history is in PDF statements.",
  },
  {
    id: "bank-of-america",
    name: "Bank of America",
    formats: ["csv", "qfx"],
    steps: [
      "Open the account and find Download near the transactions list.",
      "Choose the file format and the date range or statement period.",
      "Download, then import the file here.",
    ],
  },
  {
    id: "wells-fargo",
    name: "Wells Fargo",
    formats: ["csv", "qfx"],
    steps: [
      "Open Account activity and choose Download account activity.",
      "Select the account, date range, and file format.",
      "Download, then import the file here.",
    ],
  },
  {
    id: "capital-one",
    name: "Capital One",
    formats: ["csv", "ofx"],
    steps: [
      "Open the account and look for Download transactions above the activity list.",
      "Choose the date range and a file type.",
      "Download, then import the file here.",
    ],
    note: "Card exports carry both a transaction and a posted date; both are kept on import.",
  },
  {
    id: "american-express",
    name: "American Express",
    formats: ["csv", "ofx", "qfx"],
    steps: [
      "Open Statements & Activity and choose the Download or Export option.",
      "Pick the statement period or a custom date range and a file type.",
      "Download, then import the file here.",
    ],
  },
  {
    id: "citi",
    name: "Citi",
    formats: ["csv", "qfx"],
    steps: [
      "Open the account activity and choose Download or Export.",
      "Select the date range and file format.",
      "Download, then import the file here.",
    ],
  },
  {
    id: "discover",
    name: "Discover",
    formats: ["csv", "qfx"],
    steps: [
      "Open Activity & Statements and choose Download Transactions.",
      "Pick the statement or date range and a file format.",
      "Download, then import the file here.",
    ],
  },
  {
    id: "sofi",
    name: "SoFi",
    formats: ["csv"],
    steps: [
      "Open the account, then Statements or Transaction history.",
      "Choose Export or Download for the period you want.",
      "Download the CSV, then import it here.",
    ],
  },
  {
    id: "ally",
    name: "Ally",
    formats: ["csv", "qfx"],
    steps: [
      "Open the account activity and choose Download transactions.",
      "Select the date range and file type.",
      "Download, then import the file here.",
    ],
  },
  {
    id: "fidelity",
    name: "Fidelity",
    formats: ["csv"],
    steps: [
      "Open Activity & Orders for the account.",
      "Choose the date range, then Download (top of the activity table).",
      "Download the CSV, then import it here.",
    ],
    note: "Brokerage exports list trades alongside cash activity; every line with an amount is imported as-is.",
  },
  {
    id: "schwab",
    name: "Charles Schwab",
    formats: ["csv"],
    steps: [
      "Open Accounts, then History for the account.",
      "Choose the date range, then Export.",
      "Download the CSV, then import it here.",
    ],
  },
  {
    id: "vanguard",
    name: "Vanguard",
    formats: ["csv", "ofx"],
    steps: [
      "Open the account and choose Download center (under Activity or the profile menu).",
      "Pick the accounts, date range, and a file format.",
      "Download, then import the file here.",
    ],
  },
  {
    id: "other",
    name: "Another bank or card",
    formats: ["csv", "ofx", "qfx"],
    steps: [
      "Sign in on the web (mobile apps rarely export) and open the account's activity or statements.",
      "Look for Download, Export, or a file-type menu — usually near the transaction list or statement period.",
      "Prefer OFX or QFX when offered (cleaner dates and ids); CSV works too.",
      "Download, then import the file here.",
    ],
  },
];

export function guideFor(id: string): ExportGuide {
  return EXPORT_GUIDES.find((guide) => guide.id === id) ?? EXPORT_GUIDES[EXPORT_GUIDES.length - 1]!;
}
