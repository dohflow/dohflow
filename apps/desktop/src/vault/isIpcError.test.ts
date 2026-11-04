import type { IpcError } from "@/bindings";

import { isIpcError } from "./useVault";

vi.mock("@/bindings", () => ({ commands: {} }));

// personal-cfo-5ie.20: a call that resolves to `IpcError | null` (or `{ id } | IpcError`) is told
// apart by the generated type's shape, never by a key the type does not have. One entry per variant
// of `IpcError`; the exhaustiveness checks below stop compiling when the generated type grows one.

const stringVariants = [
  "VaultLocked",
  "VaultUnlockFailed",
  "NewerVaultSchema",
  "VaultIntegrityCheckFailed",
  "UnsupportedVaultSchema",
  "InsufficientDiskSpace",
  "WriterPanicked",
] as const satisfies readonly Extract<IpcError, string>[];
const everyStringVariantIsListed: Extract<IpcError, string> extends (typeof stringVariants)[number] ? true : never = true;

const objectVariants: IpcError[] = [
  { Validation: "Enter a name." },
  { Persistence: "disk I/O error" },
  { Unavailable: "restoring" },
  { LoanAlreadyLinked: { bill_id: "0190b000-0000-7000-8000-000000000001", bill_name: "ACME HOME LOANS" } },
];
type ObjectVariantKeys = Extract<IpcError, object> extends infer U ? (U extends unknown ? keyof U : never) : never;
const everyObjectKeyIsListed: Exclude<ObjectVariantKeys, "Validation" | "Persistence" | "Unavailable" | "LoanAlreadyLinked"> extends never
  ? true
  : never = true;

test("the exhaustiveness checks compile", () => {
  expect(everyStringVariantIsListed && everyObjectKeyIsListed).toBe(true);
});

test.each(stringVariants.map((variant) => [variant]))("%s is an IpcError", (variant) => {
  expect(isIpcError(variant)).toBe(true);
});

test.each(objectVariants.map((variant) => [Object.keys(variant)[0]!, variant] as const))(
  "{ %s } is an IpcError",
  (_name, variant) => {
    expect(isIpcError(variant)).toBe(true);
  },
);

test("what a successful call resolves to is not an IpcError", () => {
  expect(isIpcError(null)).toBe(false);
  expect(isIpcError(undefined)).toBe(false);
  // A duplicate resolves to the new scenario's id.
  expect(isIpcError({ id: "sce-2" })).toBe(false);
  expect(isIpcError("sce-2")).toBe(false);
  expect(isIpcError(0)).toBe(false);
  expect(isIpcError(true)).toBe(false);
  expect(isIpcError([])).toBe(false);
  expect(isIpcError({})).toBe(false);
});

test("a look-alike that is not one of the generated shapes is not an IpcError", () => {
  // The shape the old check looked for.
  expect(isIpcError({ kind: "Validation", message: "x" })).toBe(false);
  expect(isIpcError({ Validation: 3 })).toBe(false);
  expect(isIpcError({ Persistence: null })).toBe(false);
  expect(isIpcError({ LoanAlreadyLinked: "ACME" })).toBe(false);
  expect(isIpcError({ LoanAlreadyLinked: { bill_id: "x" } })).toBe(false);
  expect(isIpcError({ Validation: "x", id: "sce-2" })).toBe(false);
  expect(isIpcError("SomethingNew")).toBe(false);
});
