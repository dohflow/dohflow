import { useEffect, useMemo, useState } from "react";
import { Loader2, Plus, X } from "lucide-react";

import type {
  AccountViewDto,
  CreateAccountInput,
  RepaymentPhilosophyDto,
  SetDebtTermsInput,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { describeIpcError } from "@/vault/useVault";
import { dollarsToMinorUnits } from "@/lib/format";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { cn } from "@/lib/utils";
import { useAccounts } from "./useAccounts";
import { useDebtTerms } from "./useDebtTerms";
import { ROLE_DTO_TO_TOKEN, subtypesForRoleToken } from "./subtypes";
import {
  enteredToStoredMinor,
  figureLabelForRole,
  storedToShownMinor,
} from "./balanceSign";
import { NewLoanDialog } from "./NewLoanDialog";

/// The account-type (cashflow-role) options, in the DTO enum the create command takes.
const ACCOUNT_TYPES: { value: CreateAccountInput["cashflow_role"]; label: string }[] = [
  { value: "LiquidCash", label: "Cash / bank" },
  { value: "CreditFacility", label: "Credit card / line of credit" },
  { value: "LoanLiability", label: "Loan / mortgage" },
  { value: "InvestmentAsset", label: "Investment (brokerage, retirement, HSA, crypto)" },
  { value: "RealAsset", label: "Property / vehicle" },
];

/// The role tokens whose accounts carry debt terms (ADR 0035).
const DEBT_ROLES = new Set(["credit_facility", "loan_liability"]);

const PHILOSOPHIES: { value: RepaymentPhilosophyDto; label: string }[] = [
  { value: "unknown", label: "Not set (assume the minimum)" },
  { value: "pay_in_full", label: "Pay in full" },
  { value: "pay_statement_balance", label: "Pay the statement balance" },
  { value: "pay_current_balance", label: "Pay the current balance" },
  { value: "pay_minimum", label: "Pay the minimum" },
  { value: "pay_fixed_amount", label: "Pay a fixed amount" },
];

// ── small string↔number helpers (shared with the former DebtTermsModal) ──
function intOrNull(s: string): number | null {
  const t = s.trim();
  if (!/^\d+$/.test(t)) return null;
  return Number(t);
}
function pctToBps(s: string): number | null {
  const t = s.trim();
  if (t === "") return null;
  const n = Number(t);
  return Number.isFinite(n) ? Math.round(n * 100) : null;
}
function dollarsOrNull(s: string): number | null {
  if (s.trim() === "") return null;
  return dollarsToMinorUnits(s);
}
const bpsToPct = (bps: number | null) => (bps === null ? "" : String(bps / 100));
const minorToDollars = (m: number | null) => (m === null ? "" : String(m / 100));
const intToString = (n: number | null) => (n === null ? "" : String(n));

/// The unified account editor (ADR 0044, personal-cfo-4d8.22.5): ONE drawer for both
/// creating and editing an account, with an inline Debt section (so debt terms are
/// captured at entry, not in a second modal) and a Notes section. Replaces the separate
/// AddAccountForm + DebtTermsModal. `account === null` is create mode. Saving runs the
/// existing per-field commands in sequence (rename / subtype / debt terms / note /
/// balance); Rust stays authoritative for validation (ADR 0003). Real assets present
/// their figure as a "Value" (ADR 0044) — a label choice, not a model change.
export function AccountEditorDrawer({
  account,
  defaultCurrency,
  onClose,
}: {
  account: AccountViewDto | null;
  defaultCurrency: "USD" | "EUR";
  onClose: () => void;
}) {
  const isEdit = account !== null;
  const {
    accounts,
    createAccount,
    renameAccount,
    setSubtype,
    assertBalance,
    setAccountNote,
    setAccountLink,
  } = useAccounts();
  const {
    terms,
    loading: debtLoading,
    save: saveDebtTerms,
  } = useDebtTerms(account?.id ?? "");

  // Details.
  const [name, setName] = useState(account?.name ?? "");
  const [role, setRole] = useState<CreateAccountInput["cashflow_role"]>(() =>
    dtoRoleFromToken(account?.cashflow_role) ?? "LiquidCash",
  );
  const [subtype, setSubtypeState] = useState(account?.subtype ?? "");
  const [currency] = useState(account?.balance.currency ?? defaultCurrency);
  const [figure, setFigure] = useState(
    account
      ? minorToDollars(
          storedToShownMinor(account.cashflow_role, account.balance.minor_units),
        )
      : "",
  );
  const [notes, setNotes] = useState(account?.notes ?? "");
  // Link (ADR 0044 §5) — a real asset's financing loan.
  const [linkedLiabilityId, setLinkedLiabilityId] = useState(
    account?.linked_account_id ?? "",
  );
  const [creatingLoan, setCreatingLoan] = useState(false);

  // Debt.
  const [apr, setApr] = useState("");
  const [creditLimit, setCreditLimit] = useState("");
  const [originalPrincipal, setOriginalPrincipal] = useState("");
  const [closeDay, setCloseDay] = useState("");
  const [dueDay, setDueDay] = useState("");
  const [graceDays, setGraceDays] = useState("");
  const [philosophy, setPhilosophy] = useState<RepaymentPhilosophyDto>("unknown");
  const [fixedAmount, setFixedAmount] = useState("");
  const [minPercent, setMinPercent] = useState("");
  const [minFloor, setMinFloor] = useState("");
  const [payingSource, setPayingSource] = useState("");

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const roleToken = ROLE_DTO_TO_TOKEN[role] ?? "";
  const isDebt = DEBT_ROLES.has(roleToken);
  const isRealAsset = roleToken === "real_asset";
  const subtypeOptions = subtypesForRoleToken(roleToken);
  const liquidAccounts = useMemo(
    () => (accounts ?? []).filter((a) => a.cashflow_role === "liquid_cash" && a.active),
    [accounts],
  );
  // Real assets can be linked to the LOAN that finances them (ADR 0044 §5, 4d8.23.4) — a
  // mortgage/auto-loan, not a revolving credit card.
  const loanAccounts = useMemo(
    () => (accounts ?? []).filter((a) => a.cashflow_role === "loan_liability" && a.active),
    [accounts],
  );
  const figureLabel = figureLabelForRole(roleToken);
  // Guard the debt-terms race: while an existing debt account's terms are still
  // loading, the form is empty, so saving would overwrite real terms with nulls.
  const debtBlocked = isEdit && isDebt && debtLoading;

  // Hydrate the debt section once existing terms load (edit mode).
  useEffect(() => {
    if (!terms) return;
    setApr(bpsToPct(terms.apr_bps));
    setCreditLimit(minorToDollars(terms.credit_limit_minor));
    setOriginalPrincipal(minorToDollars(terms.original_principal_minor));
    setCloseDay(intToString(terms.statement_close_day));
    setDueDay(intToString(terms.payment_due_day));
    setGraceDays(intToString(terms.grace_period_days));
    setPhilosophy(terms.repayment_philosophy);
    setFixedAmount(minorToDollars(terms.fixed_amount_minor));
    setMinPercent(bpsToPct(terms.min_payment_percent_bps));
    setMinFloor(minorToDollars(terms.min_payment_floor_minor));
    setPayingSource(terms.paying_source_account_id ?? "");
  }, [terms]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  function debtInputFor(accountId: string): SetDebtTermsInput {
    return {
      account_id: accountId,
      apr_bps: pctToBps(apr),
      credit_limit_minor: roleToken === "credit_facility" ? dollarsOrNull(creditLimit) : null,
      original_principal_minor:
        roleToken === "loan_liability" ? dollarsOrNull(originalPrincipal) : null,
      statement_close_day: intOrNull(closeDay),
      payment_due_day: intOrNull(dueDay),
      grace_period_days: intOrNull(graceDays),
      repayment_philosophy: philosophy,
      // Only persist a fixed amount under pay_fixed_amount (ADR 0035 §1).
      fixed_amount_minor:
        philosophy === "pay_fixed_amount" ? dollarsOrNull(fixedAmount) : null,
      min_payment_percent_bps: pctToBps(minPercent),
      min_payment_floor_minor: dollarsOrNull(minFloor),
      paying_source_account_id: payingSource === "" ? null : payingSource,
      idempotency_key: mintIdempotencyKey(),
    };
  }

  async function onSave() {
    if (debtBlocked) return;
    if (name.trim() === "") {
      setError("Enter an account name.");
      return;
    }
    const figureMinor = figure.trim() === "" ? null : dollarsToMinorUnits(figure);
    if (figure.trim() !== "" && figureMinor === null) {
      setError(`Enter a valid ${figureLabel.toLowerCase()}.`);
      return;
    }
    const validSubtype = subtypeOptions.some((o) => o.value === subtype);
    const chosenSubtype = validSubtype ? subtype : null;

    setBusy(true);
    setError(null);
    const fail = (e: IpcErrorLike): boolean => {
      setBusy(false);
      setError(describeIpcError(e));
      return true;
    };

    if (!isEdit) {
      const { id, error: createErr } = await createAccount({
        name: name.trim(),
        cashflow_role: role,
        subtype: chosenSubtype,
        currency,
        flags: null,
        opening_balance:
          figureMinor === null
            ? null
            : {
                minor_units: enteredToStoredMinor(roleToken, figureMinor),
                currency,
              },
        idempotency_key: mintIdempotencyKey(),
      });
      if (createErr || id === null) {
        if (createErr) return void fail(createErr);
        setBusy(false);
        return;
      }
      if (isDebt) {
        const e = await saveDebtTerms(debtInputFor(id));
        if (e) return void fail(e);
      }
      if (notes.trim() !== "") {
        const e = await setAccountNote(id, notes.trim());
        if (e) return void fail(e);
      }
      if (isRealAsset && linkedLiabilityId !== "") {
        const e = await setAccountLink(id, linkedLiabilityId);
        if (e) return void fail(e);
      }
    } else {
      const acc = account;
      if (name.trim() !== acc.name) {
        const e = await renameAccount(acc.id, name.trim());
        if (e) return void fail(e);
      }
      if (chosenSubtype !== (acc.subtype ?? null)) {
        const e = await setSubtype(acc.id, chosenSubtype);
        if (e) return void fail(e);
      }
      const storedFigureMinor =
        figureMinor === null ? null : enteredToStoredMinor(roleToken, figureMinor);
      if (storedFigureMinor !== null && storedFigureMinor !== acc.balance.minor_units) {
        const { error: e } = await assertBalance({
          account_id: acc.id,
          amount: { minor_units: storedFigureMinor, currency: acc.balance.currency },
          as_of_date: new Date().toLocaleDateString("en-CA"),
        });
        if (e) return void fail(e);
      }
      if ((notes.trim() || null) !== (acc.notes ?? null)) {
        const e = await setAccountNote(acc.id, notes.trim() || null);
        if (e) return void fail(e);
      }
      if (isDebt) {
        const e = await saveDebtTerms(debtInputFor(acc.id));
        if (e) return void fail(e);
      }
      if (isRealAsset && (linkedLiabilityId || null) !== (acc.linked_account_id ?? null)) {
        const e = await setAccountLink(acc.id, linkedLiabilityId || null);
        if (e) return void fail(e);
      }
    }
    setBusy(false);
    onClose();
  }

  return (
    <div className="fixed inset-0 z-50 flex justify-end" role="dialog" aria-modal="true"
      aria-label={isEdit ? `Edit ${account.name}` : "New account"}>
      <div className="absolute inset-0 bg-foreground/40" aria-hidden onClick={onClose} />
      <div className="relative flex h-full w-full max-w-2xl flex-col border-l bg-background shadow-xl">
        <div className="flex items-center justify-between border-b px-5 py-4">
          <h2 className="font-semibold">{isEdit ? "Edit account" : "New account"}</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="size-5" aria-hidden />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto px-5 py-4">
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void onSave();
            }}
            className="grid grid-cols-1 gap-x-6 gap-y-5 sm:grid-cols-2"
          >
            <Section
              title="Details"
              className={isDebt ? "sm:col-span-1" : "sm:col-span-2"}
            >
              <Field label="Account name">
                <Input
                  autoFocus
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="e.g. Joint checking"
                />
              </Field>
              <div className="grid grid-cols-2 gap-3">
                <Field label="Type">
                  <NativeSelect
                    value={role}
                    onChange={(e) => {
                      setRole(e.target.value as CreateAccountInput["cashflow_role"]);
                      setSubtypeState("");
                    }}
                    disabled={isEdit}
                  >
                    {ACCOUNT_TYPES.map((t) => (
                      <option key={t.value} value={t.value}>
                        {t.label}
                      </option>
                    ))}
                  </NativeSelect>
                </Field>
                {subtypeOptions.length > 0 && (
                  <Field label="Subtype">
                    <NativeSelect
                      value={subtype}
                      onChange={(e) => setSubtypeState(e.target.value)}
                    >
                      <option value="">— None —</option>
                      {subtypeOptions.map((o) => (
                        <option key={o.value} value={o.value}>
                          {o.label}
                        </option>
                      ))}
                    </NativeSelect>
                  </Field>
                )}
              </div>
              <Field label={isEdit ? figureLabel : `Opening ${figureLabel.toLowerCase()}`}>
                <Input
                  inputMode="decimal"
                  value={figure}
                  onChange={(e) => setFigure(e.target.value)}
                  placeholder="0.00"
                />
              </Field>
              {isRealAsset && (
                <p className="text-xs text-muted-foreground">
                  The current estimated value of this asset — update it whenever it changes.
                </p>
              )}
              {isDebt && (
                <p className="text-xs text-muted-foreground">
                  Enter what you currently owe as a positive number — the balance you see on
                  your statement.
                </p>
              )}
            </Section>

            {isDebt && (
              <Section title="Debt">
                <p className="text-xs text-muted-foreground">
                  These settings shape how this debt appears in your forecast.
                </p>
                <div className="grid grid-cols-2 gap-3">
                  <Field label="APR (%)">
                    <Input inputMode="decimal" value={apr} onChange={(e) => setApr(e.target.value)} placeholder="0.00" />
                  </Field>
                  {roleToken === "credit_facility" ? (
                    <Field label="Credit limit">
                      <Input inputMode="decimal" value={creditLimit} onChange={(e) => setCreditLimit(e.target.value)} placeholder="0.00" />
                    </Field>
                  ) : (
                    <Field label="Original principal">
                      <Input inputMode="decimal" value={originalPrincipal} onChange={(e) => setOriginalPrincipal(e.target.value)} placeholder="0.00" />
                    </Field>
                  )}
                  <Field label="Statement close day">
                    <Input inputMode="numeric" value={closeDay} onChange={(e) => setCloseDay(e.target.value)} placeholder="1–31" />
                  </Field>
                  <Field label="Payment due day">
                    <Input inputMode="numeric" value={dueDay} onChange={(e) => setDueDay(e.target.value)} placeholder="1–31" />
                  </Field>
                  <Field label="Grace period (days)">
                    <Input inputMode="numeric" value={graceDays} onChange={(e) => setGraceDays(e.target.value)} placeholder="e.g. 25" />
                  </Field>
                </div>
                <Field label="Repayment philosophy">
                  <NativeSelect value={philosophy} onChange={(e) => setPhilosophy(e.target.value as RepaymentPhilosophyDto)}>
                    {PHILOSOPHIES.map((p) => (
                      <option key={p.value} value={p.value}>{p.label}</option>
                    ))}
                  </NativeSelect>
                </Field>
                {philosophy === "pay_fixed_amount" && (
                  <Field label="Monthly payment">
                    <Input inputMode="decimal" value={fixedAmount} onChange={(e) => setFixedAmount(e.target.value)} placeholder="0.00" />
                  </Field>
                )}
                <div className="grid grid-cols-2 gap-3">
                  <Field label="Min payment (% of balance)">
                    <Input inputMode="decimal" value={minPercent} onChange={(e) => setMinPercent(e.target.value)} placeholder="e.g. 2" />
                  </Field>
                  <Field label="Min payment floor">
                    <Input inputMode="decimal" value={minFloor} onChange={(e) => setMinFloor(e.target.value)} placeholder="0.00" />
                  </Field>
                </div>
                <Field label="Paid from">
                  <NativeSelect value={payingSource} onChange={(e) => setPayingSource(e.target.value)}>
                    <option value="">No account chosen</option>
                    {liquidAccounts.map((a) => (
                      <option key={a.id} value={a.id}>{a.name}</option>
                    ))}
                  </NativeSelect>
                </Field>
              </Section>
            )}

            {isRealAsset && (
              <Section title="Link" className="sm:col-span-2">
                <Field label="Financed by">
                  <div className="flex gap-2">
                    <NativeSelect
                      aria-label="Financed by"
                      className="flex-1"
                      value={linkedLiabilityId}
                      onChange={(e) => setLinkedLiabilityId(e.target.value)}
                    >
                      <option value="">Not financed by a loan</option>
                      {loanAccounts.map((a) => (
                        <option key={a.id} value={a.id}>
                          {a.name}
                        </option>
                      ))}
                    </NativeSelect>
                    <Button
                      type="button"
                      variant="outline"
                      className="shrink-0"
                      onClick={() => setCreatingLoan(true)}
                    >
                      <Plus aria-hidden />
                      New loan
                    </Button>
                  </div>
                </Field>
                <p className="text-xs text-muted-foreground">
                  Link this asset to the loan that finances it (e.g. a house to its
                  mortgage). This only shows the connection — it doesn&apos;t change any
                  balances or your net worth.
                </p>
              </Section>
            )}

            <Section title="Notes" className="sm:col-span-2">
              <textarea
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
                rows={3}
                placeholder="Anything you want to remember about this account."
                className={cn(
                  "w-full rounded-md border border-input bg-background px-3 py-2 text-sm",
                  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                )}
              />
            </Section>

            {error && (
              <p role="alert" className="text-sm text-loss">
                {error}
              </p>
            )}
          </form>
        </div>

        <div className="flex items-center justify-end gap-2 border-t px-5 py-4">
          <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button
            type="button"
            onClick={() => void onSave()}
            disabled={busy || debtBlocked}
          >
            {busy && <Loader2 className="animate-spin" aria-hidden />}
            {isEdit ? "Save account" : "Add account"}
          </Button>
        </div>
      </div>
      {creatingLoan && (
        <NewLoanDialog
          currency={currency}
          onCreated={(id) => setLinkedLiabilityId(id)}
          onClose={() => setCreatingLoan(false)}
        />
      )}
    </div>
  );
}

type IpcErrorLike = Parameters<typeof describeIpcError>[0];

/// `AccountViewDto.cashflow_role` token → the create command's DTO enum value.
function dtoRoleFromToken(
  token: string | undefined,
): CreateAccountInput["cashflow_role"] | null {
  const entry = Object.entries(ROLE_DTO_TO_TOKEN).find(([, t]) => t === token);
  return (entry?.[0] as CreateAccountInput["cashflow_role"] | undefined) ?? null;
}

function Section({
  title,
  className,
  children,
}: {
  title: string;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <section className={cn("flex flex-col gap-3", className)}>
      <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </h3>
      {children}
    </section>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-sm font-medium">{label}</span>
      {children}
    </label>
  );
}
