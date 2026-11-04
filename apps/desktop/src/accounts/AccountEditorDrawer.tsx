import { useEffect, useMemo, useRef, useState } from "react";
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
import { BALANCE_OUTCOME_UNKNOWN, BALANCE_SAVED_UNCONFIRMED } from "./balanceMessages";
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

/// What the account is known to hold. Edit mode starts from the account being edited; create mode
/// starts empty and takes the account's values from the form once `createAccount` succeeds, so
/// from then on the drawer is editing THAT account and a retry cannot create a second one.
type Baseline = {
  id: string;
  name: string;
  subtype: string | null;
  balanceMinor: number;
  currency: string;
  notes: string | null;
  linkedId: string | null;
};

const baselineOf = (a: AccountViewDto): Baseline => ({
  id: a.id,
  name: a.name,
  subtype: a.subtype ?? null,
  balanceMinor: a.balance.minor_units,
  currency: a.balance.currency,
  notes: a.notes ?? null,
  linkedId: a.linked_account_id ?? null,
});

/// What a failed save tells the person, in three buckets: the steps that are stored, the step the
/// app refused (`failed`, with its reason, so it did not save), or the step whose call threw
/// (`unconfirmed`, so nobody knows whether it saved), and the steps never tried. Exactly one of
/// `failed` and `unconfirmed` is set. There is no rollback, so nothing here offers an undo.
type PartialSave = {
  saved: string[];
  failed: { label: string; reason: string } | null;
  unconfirmed: string | null;
  untried: string[];
  /// The balance was stored but its new figure could not be read back. It is in `saved`.
  balanceUnconfirmed: boolean;
};

/// A step whose call threw: the app can't say whether it saved. Every step after the account is
/// created sets a value, so trying it again is safe.
const STEP_OUTCOME_UNKNOWN = "Couldn’t confirm whether this saved.";

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
  // Progress across attempts at one save (g3m.35): the account as stored so far, the steps that
  // saved, and the one idempotency key for creating it.
  const [base, setBase] = useState<Baseline | null>(account ? baselineOf(account) : null);
  const [savedSteps, setSavedSteps] = useState<string[]>([]);
  const [partial, setPartial] = useState<PartialSave | null>(null);
  const createKey = useRef<{ identity: string; key: string } | null>(null);
  // The balance step's draft: one key per account, figure, date and currency (personal-cfo-g3m.40).
  // A retry of an unknown outcome reuses it so the append-only assertion cannot be written twice.
  const balanceKey = useRef<{ identity: string; key: string } | null>(null);
  // The account exists, whether it was opened for editing or was created by this drawer.
  const exists = base !== null;

  // Every step is stored and only the new balance could not be read back: nothing is left to retry,
  // so the one action is to finish (it sends nothing; the baseline already holds the saved figures).
  const everythingSaved =
    partial !== null &&
    partial.failed === null &&
    partial.unconfirmed === null &&
    partial.untried.length === 0;

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

  // Nothing closes the drawer while a save is in flight: the steps would finish unseen.
  const requestClose = () => {
    if (!busy) onClose();
  };
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape" && !busy) onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, busy]);

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
    setPartial(null);
    const storedFigureMinor =
      figureMinor === null
        ? null
        : enteredToStoredMinor(roleToken, figureMinor);
    const saved = [...savedSteps];

    let cur = base;
    if (cur === null) {
      const input = {
        name: name.trim(),
        cashflow_role: role,
        subtype: chosenSubtype,
        currency,
        flags: null,
        opening_balance:
          storedFigureMinor === null
            ? null
            : { minor_units: storedFigureMinor, currency },
      };
      // One key per draft: a retry of the same account replays instead of creating another.
      const identity = JSON.stringify(input);
      if (createKey.current?.identity !== identity) {
        createKey.current = { identity, key: mintIdempotencyKey() };
      }
      let created: Awaited<ReturnType<typeof createAccount>>;
      try {
        created = await createAccount({
          ...input,
          idempotency_key: createKey.current.key,
        });
      } catch {
        setBusy(false);
        setError(STEP_OUTCOME_UNKNOWN);
        return;
      }
      if (created.error || created.id === null) {
        setBusy(false);
        if (created.error) setError(describeIpcError(created.error));
        return;
      }
      cur = {
        id: created.id,
        name: input.name,
        subtype: chosenSubtype,
        balanceMinor: storedFigureMinor ?? 0,
        currency,
        notes: null,
        linkedId: null,
      };
      saved.push("Account");
      setBase(cur);
      setSavedSteps(saved);
    }

    const acc: Baseline = { ...cur };
    type Step = {
      label: string;
      run: () => Promise<IpcErrorLike | null>;
      /// What a lone step that threw says, when it is more specific than the generic sentence.
      unknown?: string;
    };
    const steps: Step[] = [];
    let balanceUnconfirmed = false;
    if (name.trim() !== acc.name) {
      steps.push({
        label: "Name",
        run: async () => {
          const e = await renameAccount(acc.id, name.trim());
          if (!e) acc.name = name.trim();
          return e;
        },
      });
    }
    if (chosenSubtype !== acc.subtype) {
      steps.push({
        label: "Subtype",
        run: async () => {
          const e = await setSubtype(acc.id, chosenSubtype);
          if (!e) acc.subtype = chosenSubtype;
          return e;
        },
      });
    }
    if (storedFigureMinor !== null && storedFigureMinor !== acc.balanceMinor) {
      steps.push({
        label: figureLabel,
        unknown: BALANCE_OUTCOME_UNKNOWN,
        run: async () => {
          const asOf = new Date().toLocaleDateString("en-CA");
          const identity = `${acc.id}|${storedFigureMinor}|${asOf}|${acc.currency}`;
          if (balanceKey.current?.identity !== identity) {
            balanceKey.current = { identity, key: mintIdempotencyKey() };
          }
          // A call that throws leaves the key in place: the next try is the same draft.
          const { result, error: e } = await assertBalance({
            account_id: acc.id,
            amount: { minor_units: storedFigureMinor, currency: acc.currency },
            as_of_date: asOf,
            idempotency_key: balanceKey.current.key,
          });
          // Answered either way, so the draft is closed: a refusal saved nothing (the next try is a
          // new draft) and a save is final.
          balanceKey.current = null;
          if (!e) {
            acc.balanceMinor = storedFigureMinor;
            // Stored, but the new figure could not be read back: say so, never "not saved".
            if (result?.balance === null) balanceUnconfirmed = true;
          }
          return e;
        },
      });
    }
    if ((notes.trim() || null) !== acc.notes) {
      steps.push({
        label: "Notes",
        run: async () => {
          const e = await setAccountNote(acc.id, notes.trim() || null);
          if (!e) acc.notes = notes.trim() || null;
          return e;
        },
      });
    }
    if (isDebt) {
      steps.push({
        label: "Debt",
        run: async () => {
          // Debt and a link never share an account type, so no saved step follows this one.
          return saveDebtTerms(debtInputFor(acc.id));
        },
      });
    }
    if (isRealAsset && (linkedLiabilityId || null) !== acc.linkedId) {
      steps.push({
        label: "Link",
        run: async () => {
          const e = await setAccountLink(acc.id, linkedLiabilityId || null);
          if (!e) acc.linkedId = linkedLiabilityId || null;
          return e;
        },
      });
    }

    for (const [i, step] of steps.entries()) {
      let e: IpcErrorLike | null;
      let threw = false;
      try {
        e = await step.run();
      } catch {
        e = null;
        threw = true;
      }
      if (e || threw) {
        setBase(acc);
        setSavedSteps(saved);
        setBusy(false);
        if (saved.length === 0) {
          setError(threw ? (step.unknown ?? STEP_OUTCOME_UNKNOWN) : describeIpcError(e as IpcErrorLike));
        } else {
          setPartial({
            saved,
            failed: threw
              ? null
              : { label: step.label, reason: describeIpcError(e as IpcErrorLike) },
            unconfirmed: threw ? step.label : null,
            untried: steps.slice(i + 1).map((t) => t.label),
            balanceUnconfirmed,
          });
        }
        return;
      }
      saved.push(step.label);
    }
    setBusy(false);
    if (balanceUnconfirmed) {
      // Everything is stored, but the person has not seen the new balance: stay open and say so.
      setBase(acc);
      setSavedSteps(saved);
      setPartial({
        saved,
        failed: null,
        unconfirmed: null,
        untried: [],
        balanceUnconfirmed,
      });
      return;
    }
    onClose();
  }

  return (
    <div className="fixed inset-0 z-50 flex justify-end" role="dialog" aria-modal="true"
      aria-label={exists ? `Edit ${base.name}` : "New account"}>
      <div className="absolute inset-0 bg-foreground/40" aria-hidden onClick={requestClose} />
      <div className="relative flex h-full w-full max-w-2xl flex-col border-l bg-background shadow-xl">
        <div className="flex items-center justify-between border-b px-5 py-4">
          <h2 className="font-semibold">{exists ? "Edit account" : "New account"}</h2>
          <button
            type="button"
            onClick={requestClose}
            disabled={busy}
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
            // A change after a finished save is new work: the result no longer describes the form,
            // so the drawer goes back to normal saving instead of treating the next press as done.
            onChange={() => {
              if (everythingSaved) setPartial(null);
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
                    disabled={exists}
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
              <Field label={exists ? figureLabel : `Opening ${figureLabel.toLowerCase()}`}>
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
            {partial && (
              <div role="alert" className="flex flex-col gap-1 text-sm sm:col-span-2">
                {(partial.failed !== null || partial.unconfirmed !== null) && (
                  <p className="font-medium text-loss">
                    {partial.unconfirmed !== null
                      ? account
                        ? "Some changes saved and some couldn’t be confirmed."
                        : "The account was added, but not everything could be confirmed."
                      : account
                        ? "Some changes saved and some didn’t."
                        : "The account was added, but not everything saved."}
                  </p>
                )}
                <p>Saved: {partial.saved.join(", ")}.</p>
                {partial.failed && (
                  <p className="text-loss">
                    Didn’t save: {partial.failed.label}. {partial.failed.reason}
                  </p>
                )}
                {partial.unconfirmed !== null && (
                  <p className="text-loss">
                    Couldn’t confirm: {partial.unconfirmed}. It may or may not have saved.
                  </p>
                )}
                {partial.untried.length > 0 && <p>Not tried yet: {partial.untried.join(", ")}.</p>}
                {partial.balanceUnconfirmed && <p>{BALANCE_SAVED_UNCONFIRMED}</p>}
              </div>
            )}
          </form>
        </div>

        <div className="flex items-center justify-end gap-2 border-t px-5 py-4">
          <Button type="button" variant="ghost" onClick={requestClose} disabled={busy}>
            Cancel
          </Button>
          <Button
            type="button"
            // Done closes without running the save path: the save path rebuilds its steps from the
            // form (a debt account always has its terms step), so it would write again.
            onClick={() => (everythingSaved ? requestClose() : void onSave())}
            disabled={busy || debtBlocked}
          >
            {busy && <Loader2 className="animate-spin" aria-hidden />}
            {everythingSaved ? "Done" : partial ? "Try again" : exists ? "Save account" : "Add account"}
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
