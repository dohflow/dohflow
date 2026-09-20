//! The typed Tauri command surface (personal-cfo-40t).
//!
//! Each operation is a thin `#[tauri::command] #[specta::specta]` wrapper around
//! a plain `*_impl` function that takes `&AppState`. The split keeps the command
//! bodies trivial *and* lets integration tests exercise the real logic against a
//! temp-vault kernel without constructing a `tauri::State` (which only Tauri's
//! DI can build). Commands carry **no business logic** — they validate input
//! shape, build a kernel command + provenance, dispatch, and map the result.

use chrono::NaiveDate;
use finance_kernel::{
    all_presets, detect_best, plugin_by_id, preset_by_id, ActorType, ApplyScenario, ArchiveAccount,
    ArchiveCategory, ArchiveIncomeSource, ArchiveRecurringBill, AttachSourceRecord, CategoryId,
    CommandEnvelope, CommandMeta, CommitStaged, ConfirmObligationEarly,
    ConvertUnexplainedToTransaction, CreateAccount, CreateCategory, CreateIncomeSource,
    CreateRecurringBill, CreateRecurringTransfer, CreateSourceBatch, CreateTag, DeleteIncomeSource,
    DeleteRecurringBill, DeleteRecurringTransfer, DismissInboxItem, DismissRecurringSuggestion,
    Kernel, MarkReviewed, Money, MoveCategory, NewScenario, ParserHints, ParserInput, ParserLimits,
    RecategorizeTransaction, RecurringEventId, RecurringTransferId, ReinstateAccount,
    ReinstateCategory, RestoreIncomeSource, RestoreRecurringBill, RevertScenarioApply,
    ScenarioStatus, SetAccountLink, SetAccountNote, SetAccountSubtype, SetBillAutopay,
    SetCardStatementBalance, SetDebtTerms, SetNote, SetSplits, SetTags, SkipStaged,
    SnoozeInboxItem, SourceBatchId, SourceRecordId, SpendFilters, TagId, TransactionId,
    UnconfirmObligation, UpdateAccount, UpdateBatchState, UpdateCategory, UpdateIncomeSource,
    UpdateRecurringBill, VaultController, VoidTransaction, COMFORT_BAND_UPPER_KEY,
    MINIMUM_CASH_FLOOR_KEY,
};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::ipc::dto::{
    parse_account_id, parse_attachment_id, parse_category_id, parse_currency,
    parse_income_source_id, parse_opt_uuid, parse_recurring_event_id, parse_recurring_transfer_id,
    parse_source_batch_id, parse_staged_transaction_id, parse_subtype, parse_tag_id,
    parse_transaction_id, AccountViewDto, ApplyUpdateResultDto, AssertBalanceInput,
    AssertBalanceResult, AssumptionEventDto, AttachSourceRecordInput, AttachSourceRecordResult,
    AttachmentDto, BandDriftSignalDto, BatchResultDto, BuildInfoDto, CapabilityUnlockDto,
    CardStatementForecastDto, CardStatementHistoryDto, CashAvailabilityDto, CashFlowHistoryDto,
    CashTiersDto, CategoryDto, ChangePasswordInput, CloneScenarioInput, ColumnMappingDto,
    ComfortBandDto, ConfirmObligationEarlyInput, CreateAccountInput, CreateAccountResult,
    CreateCategoryInput, CreateCategoryResult, CreateForecastAssumptionInput,
    CreateIncomeSourceInput, CreateManualFutureEntryInput, CreateRecurringBillInput,
    CreateRecurringBillResult, CreateRecurringTransferInput, CreateRecurringTransferResult,
    CreateScenarioInput, CreateSourceBatchInput, CreateSourceBatchResult, CreateTagResult,
    DebtPayoffPlanDto, DebtTermsDto, DismissRecurringSuggestionInput, ForecastReadinessDto,
    ForecastViewDto, ImportBatchInput, ImportedTransactionFieldsDto, IncomeSourceDto,
    LoanDoubleCountWarningDto, ManualFutureEntryDto, MoneyDto, MoneyInboxItemDto,
    MoveCategoryInput, MultiSeriesForecastDto, MutationResult, RecordTransactionInput,
    RecordTransactionResult, RecordTransferInput, RecurringBillDto, RecurringBillOccurrenceDto,
    RecurringCandidateDto, RecurringTransferDto, ReleaseUpdateFailureKind, ScenarioDto,
    SetBillAutopayInput, SetCardStatementBalanceInput, SetDebtTermsInput, SetScenarioExpiryInput,
    SourcePresetDto, SpendBreakdownDto, SpendByCategoryInput, SplitLineDto, SplitLineInputDto,
    TagViewDto, TransactionPageDto, TransactionPageInput, TransactionRowDto,
    UnconfirmObligationInput, UnconfirmedOccurrenceDto, UpdateAccountInput, UpdateBatchStateInput,
    UpdateCategoryInput, UpdateIncomeSourceInput, UpdateManualFutureEntryInput,
    UpdateRecurringBillInput, UpdateScenarioInput, UpdateStatusDto, VaultHealthDto, VaultListDto,
    VaultStatusDto, VaultSummaryDto,
};
use crate::ipc::IpcError;
use crate::state::AppState;
use crate::vault_registry::VaultEntry;

/// Resolve the effective idempotency key: a blank one becomes a fresh UUIDv7 (so
/// keyless calls are still safe and never collide), a supplied one is used verbatim.
fn resolve_idempotency_key(idempotency_key: &str) -> String {
    if idempotency_key.trim().is_empty() {
        Uuid::now_v7().to_string()
    } else {
        idempotency_key.to_owned()
    }
}

/// Build provenance metadata for a single user action. A blank idempotency key
/// is replaced with a fresh UUIDv7 so retries are still safe by default.
fn user_meta(idempotency_key: &str) -> CommandMeta {
    user_meta_with_key(resolve_idempotency_key(idempotency_key))
}

/// Build provenance metadata from an already-resolved idempotency key — used when the
/// caller needs the effective key before dispatch (e.g. to derive a deterministic id).
fn user_meta_with_key(idempotency_key: String) -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "local-user".to_owned(),
        idempotency_key,
    }
}

/// Namespace for the deterministic record-transaction id (below). Fixed + arbitrary.
const RECORD_TRANSACTION_ID_NAMESPACE: Uuid = Uuid::from_bytes([
    0x72, 0x65, 0x63, 0x74, 0x78, 0x6e, 0x4d, 0x1a, 0x8f, 0x2c, 0x0b, 0x77, 0x14, 0x9e, 0x33, 0xa0,
]);

/// Run `f` with the open kernel, or return [`IpcError::VaultLocked`] when the
/// vault is not unlocked. The kernel is reached through the [`VaultController`].
fn with_kernel<T>(
    state: &AppState,
    f: impl FnOnce(&Kernel) -> Result<T, IpcError>,
) -> Result<T, IpcError> {
    let guard = state.lock_controller()?;
    let kernel = guard.kernel().ok_or(IpcError::VaultLocked)?;
    f(kernel)
}

// ---- vault lifecycle (personal-cfo-8v2 / -3ry) -----------------------------

/// Snapshot the controller for the frontend: the current state plus, when
/// unlocked, the account count. Never returns key material (§6.2 #19).
fn vault_status_dto(controller: &VaultController) -> Result<VaultStatusDto, IpcError> {
    let account_count = match controller.kernel() {
        Some(kernel) => Some(u32::try_from(kernel.account_count()?).unwrap_or(u32::MAX)),
        None => None,
    };
    Ok(VaultStatusDto {
        state: controller.state().into(),
        account_count,
    })
}

/// The current vault status — the frontend calls this on launch to decide which
/// screen to show.
pub fn vault_status_impl(state: &AppState) -> Result<VaultStatusDto, IpcError> {
    let guard = state.lock_controller()?;
    vault_status_dto(&guard)
}

#[tauri::command]
#[specta::specta]
pub fn vault_status(state: tauri::State<'_, AppState>) -> Result<VaultStatusDto, IpcError> {
    vault_status_impl(state.inner())
}

/// The canonical no-password-reset warning (ADR 0002 / personal-cfo-n7bo).
/// Onboarding renders this **verbatim**; the const in `vault-crypto` is the single
/// source of truth, so the frontend fetches it here rather than re-typing the
/// copy. Takes no state — it is available before a vault exists.
#[tauri::command]
#[specta::specta]
pub fn no_reset_warning() -> String {
    finance_kernel::CANONICAL_NO_RESET_WARNING.to_owned()
}

/// Record the user's acknowledgement of the no-password-reset warning as an
/// immutable audit event (ADR 0002 / personal-cfo-n7bo). Onboarding calls this
/// once the user checks the confirmation box (which gates vault creation in the
/// UI) on the freshly created, unlocked vault.
pub fn acknowledge_no_reset_warning_impl(state: &AppState) -> Result<(), IpcError> {
    with_kernel(state, |kernel| {
        kernel.acknowledge_no_reset_warning(&user_meta(""))?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn acknowledge_no_reset_warning(state: tauri::State<'_, AppState>) -> Result<(), IpcError> {
    acknowledge_no_reset_warning_impl(state.inner())
}

/// The household timezone to write into a freshly created vault, if the OS could name one
/// (`personal-cfo-q329`, ADR 0021 addendum): the machine's IANA zone is an acceptable
/// *initial default* even though ADR 0021 §1 forbids it as the *authoritative* value — the
/// stored `vault_metadata.household_timezone` stays authoritative from here on, changeable
/// any time via `set_household_timezone`. `None` when capture failed; the caller leaves the
/// vault at the `UTC` default `ensure_vault_metadata` already inserted. `captured` is a
/// parameter, not a direct `iana_time_zone::get_timezone()` call, so this is testable
/// without touching the OS — validity (a real `chrono_tz::Tz` name) is checked downstream by
/// `set_household_timezone` itself, not duplicated here.
fn resolve_initial_household_timezone(
    captured: Result<String, iana_time_zone::GetTimezoneError>,
) -> Option<String> {
    captured.ok()
}

/// Best-effort: point a freshly created vault's household timezone at the machine's IANA
/// zone instead of leaving it at the `UTC` default (`personal-cfo-q329`). Mirrors
/// `unlock_vault_impl`'s "extra step after the controller call succeeds, logged not
/// propagated" shape — never fails vault creation itself; a bad or unavailable capture
/// just leaves `UTC`, fixable any time from the Household settings card.
fn set_machine_timezone_on_create(kernel: &Kernel) {
    let Some(tz) = resolve_initial_household_timezone(iana_time_zone::get_timezone()) else {
        return;
    };
    if let Err(err) = kernel.set_household_timezone(&tz) {
        tracing::warn!(
            error = %err,
            tz,
            "machine-detected household timezone was not a valid IANA name; leaving UTC",
        );
    }
}

/// Create a brand-new vault protected by `password` and leave it unlocked. The
/// password is wrapped in [`Zeroizing`] so this last plaintext copy on the Rust
/// side is scrubbed when the call returns.
pub fn create_vault_impl(state: &AppState, password: String) -> Result<VaultStatusDto, IpcError> {
    let password = Zeroizing::new(password);
    let mut guard = state.lock_controller()?;
    guard.create(password.as_bytes())?;
    if let Some(kernel) = guard.kernel() {
        set_machine_timezone_on_create(kernel);
    }
    vault_status_dto(&guard)
}

#[tauri::command]
#[specta::specta]
pub fn create_vault(
    state: tauri::State<'_, AppState>,
    password: String,
) -> Result<VaultStatusDto, IpcError> {
    create_vault_impl(state.inner(), password)
}

/// Unlock the existing vault with `password`. A wrong password returns
/// [`IpcError::VaultUnlockFailed`] and leaves the vault locked.
pub fn unlock_vault_impl(state: &AppState, password: String) -> Result<VaultStatusDto, IpcError> {
    let password = Zeroizing::new(password);
    let mut guard = state.lock_controller()?;
    guard.unlock(password.as_bytes())?;
    // Daily-on-open: persist a forecast run so actualization (ADR 0026 §9/§15,
    // personal-cfo-5ie.3) accrues a time-spanning history. Deduped on
    // (input content_hash, day) inside the kernel, so repeated opens with
    // unchanged inputs write nothing. Best-effort — a persistence failure must
    // not block the user from opening their vault.
    if let Some(kernel) = guard.kernel() {
        if let Err(err) = kernel.persist_daily_forecast() {
            tracing::warn!(error = %err, "daily forecast persist on open failed");
        }
        // Score past persisted runs against realized transactions (ADR 0026 §9/§17,
        // personal-cfo-46jq) so the actuals-backed-recurrence readiness factor (nxgx)
        // reflects reality. Best-effort, like the persist above — never block the open.
        if let Err(err) = kernel.actualize_forecasts() {
            tracing::warn!(error = %err, "forecast actualization on open failed");
        }
        // Then backtest those actuals into a per-vault MAPE (ADR 0026 §18, personal-cfo-nxgx)
        // for the "Forecast accuracy" readiness factor. Consumes the actualize above, so it
        // runs after it; also best-effort.
        if let Err(err) = kernel.backtest_forecasts() {
            tracing::warn!(error = %err, "forecast backtest on open failed");
        }
    }
    vault_status_dto(&guard)
}

#[tauri::command]
#[specta::specta]
pub fn unlock_vault(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    password: String,
) -> Result<VaultStatusDto, IpcError> {
    let status = unlock_vault_impl(state.inner(), password)?;
    // Sync-on-open (personal-cfo-gglk, ADR 0060 §4): fire-and-forget so the
    // network NEVER blocks the unlock; debounced inside; outcomes land on the
    // connection rows for the health surface. Lives in the wrapper — the impl
    // stays synchronous and test-drivable without an AppHandle.
    tauri::async_runtime::spawn(async move {
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<AppState>();
            if let Err(err) = connector_auto_sync_impl(&state, connector_core::connector_by_id) {
                tracing::warn!(error = %err, "connector auto-sync on open failed");
            }
        })
        .await;
    });
    Ok(status)
}

/// Lock the vault, dropping (and zeroizing) the in-memory DEK.
pub fn lock_vault_impl(state: &AppState) -> Result<VaultStatusDto, IpcError> {
    let mut guard = state.lock_controller()?;
    guard.lock()?;
    vault_status_dto(&guard)
}

#[tauri::command]
#[specta::specta]
pub fn lock_vault(state: tauri::State<'_, AppState>) -> Result<VaultStatusDto, IpcError> {
    lock_vault_impl(state.inner())
}

/// Minimum length for a new master password. Mirrors `CreateVaultScreen`'s
/// `MIN_LENGTH` so the boundary check and the UI hint agree.
const MIN_NEW_PASSWORD_CHARS: usize = 8;

/// Change the vault master password (personal-cfo-zxq): verify the current
/// password against the envelope on disk, then atomically rewrap the DEK under
/// a KEK derived from the new password (fresh salt, current KDF profile). The
/// DEK — and therefore the encrypted database — never changes, and the vault
/// stays unlocked. A wrong current password returns
/// [`IpcError::VaultUnlockFailed`] with the vault untouched. Both passwords are
/// wrapped in [`Zeroizing`] so their plaintext copies are scrubbed on return.
pub fn change_password_impl(
    state: &AppState,
    input: ChangePasswordInput,
) -> Result<VaultStatusDto, IpcError> {
    let old_password = Zeroizing::new(input.old_password);
    let new_password = Zeroizing::new(input.new_password);
    if new_password.chars().count() < MIN_NEW_PASSWORD_CHARS {
        return Err(IpcError::Validation(format!(
            "the new password must be at least {MIN_NEW_PASSWORD_CHARS} characters"
        )));
    }
    let mut guard = state.lock_controller()?;
    guard.change_password(old_password.as_bytes(), new_password.as_bytes())?;
    vault_status_dto(&guard)
}

#[tauri::command]
#[specta::specta]
pub fn change_password(
    state: tauri::State<'_, AppState>,
    input: ChangePasswordInput,
) -> Result<VaultStatusDto, IpcError> {
    change_password_impl(state.inner(), input)
}

/// Permanently delete the active vault (personal-cfo-j0cg.5): wipe its data, drop its registry
/// entry, and fall back to another registered vault (locked) if one remains, else the create
/// screen. Recoverable only by restoring an encrypted backup. Requires the vault to be unlocked.
pub fn delete_vault_impl(state: &AppState) -> Result<VaultStatusDto, IpcError> {
    let mut controller = state.lock_controller()?;
    controller.delete()?;

    // Maintain the multi-vault registry (only in multi-vault mode — the single-vault/test path has
    // no root): drop the just-deleted active entry and fall back to another *present* vault, if any.
    if let Some(root) = state.vaults_root() {
        let root = root.to_path_buf();
        let mut registry = state.lock_registry()?;
        if let Some(active) = registry.active.take() {
            registry.vaults.retain(|v| v.id != active);
        }
        // Only fall back to a vault whose files still exist, so we never land the app on a phantom
        // (missing/corrupt) vault; if none remain, stay on the create screen (NoVault).
        registry.active = registry
            .vaults
            .iter()
            .find(|v| root.join(&v.path).exists())
            .map(|v| v.id);
        if let Some(path) = registry.active_path(&root) {
            controller.switch_to(path)?; // the fallback vault, now locked
        }
        registry
            .save(&root)
            .map_err(|e| IpcError::Persistence(format!("saving vault registry: {e}")))?;
    }
    vault_status_dto(&controller)
}

#[tauri::command]
#[specta::specta]
pub fn delete_vault(state: tauri::State<'_, AppState>) -> Result<VaultStatusDto, IpcError> {
    delete_vault_impl(state.inner())
}

/// The known vaults + which one is active (personal-cfo-j0cg.6, ADR 0042). Reads only the plaintext
/// registry — no vault need be unlocked.
pub fn list_vaults_impl(state: &AppState) -> Result<VaultListDto, IpcError> {
    let registry = state.lock_registry()?;
    let vaults = registry
        .vaults
        .iter()
        .map(|v| VaultSummaryDto {
            id: v.id.to_string(),
            name: v.name.clone(),
            is_active: registry.active == Some(v.id),
            created_at: v.created_at.clone(),
        })
        .collect();
    Ok(VaultListDto { vaults })
}

#[tauri::command]
#[specta::specta]
pub fn list_vaults(state: tauri::State<'_, AppState>) -> Result<VaultListDto, IpcError> {
    list_vaults_impl(state.inner())
}

/// Create a new, named vault under the app-managed directory and switch to it, unlocked
/// (personal-cfo-j0cg.6, ADR 0042). Locks the current vault first; on a create failure, falls back
/// to the previously-active vault so the app is never stranded on a half-made location.
pub fn create_vault_named_impl(
    state: &AppState,
    name: String,
    password: String,
) -> Result<VaultStatusDto, IpcError> {
    let password = Zeroizing::new(password);
    let root = state
        .vaults_root()
        .ok_or_else(|| IpcError::Persistence("multi-vault is not configured".to_owned()))?
        .to_path_buf();
    let id = Uuid::now_v7();
    let rel = std::path::Path::new("vaults")
        .join(id.to_string())
        .join("vault.db");
    let abs = root.join(&rel);
    std::fs::create_dir_all(abs.parent().expect("vault path has a parent"))
        .map_err(|e| IpcError::Persistence(format!("creating vault directory: {e}")))?;

    // Hold the controller AND the registry across the whole operation (controller-then-registry
    // order) so the on-disk create and the registry update are atomic: no concurrent switch/delete
    // can run, the rollback target can't go stale, and a save failure can't orphan the new vault.
    let mut controller = state.lock_controller()?;
    let mut registry = state.lock_registry()?;
    let prev_active = registry.active;
    let prev_path = registry.active_path(&root);

    controller.switch_to(abs)?; // locks the current vault, points at the fresh (empty) location
    if let Err(error) = controller.create(password.as_bytes()) {
        if let Some(prev) = prev_path {
            let _ = controller.switch_to(prev);
        }
        return Err(error.into());
    }
    if let Some(kernel) = controller.kernel() {
        set_machine_timezone_on_create(kernel);
    }

    registry.vaults.push(VaultEntry {
        id,
        name,
        path: rel,
        created_at: chrono::Utc::now().to_rfc3339(),
    });
    registry.active = Some(id);
    if let Err(error) = registry.save(&root) {
        // Don't leave an unregistered vault on disk: drop the entry, wipe the new vault, and return
        // to the previous one.
        registry.vaults.retain(|v| v.id != id);
        registry.active = prev_active;
        let _ = controller.delete();
        if let Some(prev) = prev_path {
            let _ = controller.switch_to(prev);
        }
        return Err(IpcError::Persistence(format!(
            "saving vault registry: {error}"
        )));
    }
    vault_status_dto(&controller)
}

#[tauri::command]
#[specta::specta]
pub fn create_vault_named(
    state: tauri::State<'_, AppState>,
    name: String,
    password: String,
) -> Result<VaultStatusDto, IpcError> {
    create_vault_named_impl(state.inner(), name, password)
}

/// Switch to another registered vault (personal-cfo-j0cg.6): lock the current one and point at the
/// target, which comes up **locked** — the caller unlocks it with its own password.
pub fn switch_vault_impl(state: &AppState, id: String) -> Result<VaultStatusDto, IpcError> {
    let uuid = parse_uuid(&id)?;
    let root = state
        .vaults_root()
        .ok_or_else(|| IpcError::Persistence("multi-vault is not configured".to_owned()))?
        .to_path_buf();
    let mut controller = state.lock_controller()?;
    let mut registry = state.lock_registry()?;
    let path = registry
        .vaults
        .iter()
        .find(|v| v.id == uuid)
        .map(|v| root.join(&v.path))
        .ok_or_else(|| IpcError::Validation(format!("no vault with id {id}")))?;
    controller.switch_to(path)?;
    registry.active = Some(uuid);
    registry
        .save(&root)
        .map_err(|e| IpcError::Persistence(format!("saving vault registry: {e}")))?;
    vault_status_dto(&controller)
}

#[tauri::command]
#[specta::specta]
pub fn switch_vault(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<VaultStatusDto, IpcError> {
    switch_vault_impl(state.inner(), id)
}

/// Rename a registered vault (personal-cfo-j0cg.6) and return the updated list. Touches only the
/// plaintext registry — no vault need be unlocked.
pub fn rename_vault_impl(
    state: &AppState,
    id: String,
    name: String,
) -> Result<VaultListDto, IpcError> {
    let uuid = parse_uuid(&id)?;
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(IpcError::Validation("a vault needs a name".to_owned()));
    }
    let root = state
        .vaults_root()
        .ok_or_else(|| IpcError::Persistence("multi-vault is not configured".to_owned()))?
        .to_path_buf();
    let mut registry = state.lock_registry()?;
    let entry = registry
        .vaults
        .iter_mut()
        .find(|v| v.id == uuid)
        .ok_or_else(|| IpcError::Validation(format!("no vault with id {id}")))?;
    entry.name = name;
    registry
        .save(&root)
        .map_err(|e| IpcError::Persistence(format!("saving vault registry: {e}")))?;
    let vaults = registry
        .vaults
        .iter()
        .map(|v| VaultSummaryDto {
            id: v.id.to_string(),
            name: v.name.clone(),
            is_active: registry.active == Some(v.id),
            created_at: v.created_at.clone(),
        })
        .collect();
    Ok(VaultListDto { vaults })
}

#[tauri::command]
#[specta::specta]
pub fn rename_vault(
    state: tauri::State<'_, AppState>,
    id: String,
    name: String,
) -> Result<VaultListDto, IpcError> {
    rename_vault_impl(state.inner(), id, name)
}

/// Run the vault health check (personal-cfo-n9w): per-check coherence + the overall
/// verdict. Requires an unlocked vault; failures drive the recovery wizard (5ivp).
pub fn vault_health_impl(state: &AppState) -> Result<VaultHealthDto, IpcError> {
    let guard = state.lock_controller()?;
    Ok(guard.health_check()?.into())
}

#[tauri::command]
#[specta::specta]
pub fn vault_health(state: tauri::State<'_, AppState>) -> Result<VaultHealthDto, IpcError> {
    vault_health_impl(state.inner())
}

/// Rebuild the materialized read models (transaction-display + commitments +
/// Money Inbox) from canonical tables — the recovery wizard's repair for
/// read-model drift (personal-cfo-5ivp). Deterministic and non-destructive
/// (canonical data is the source of truth). Returns the total number of rows
/// re-projected.
pub fn rebuild_read_models_impl(state: &AppState) -> Result<u32, IpcError> {
    with_kernel(state, |kernel| {
        let transaction_rows = kernel.rebuild_transaction_display()?;
        let commitment_rows = kernel.rebuild_commitments()?;
        let inbox_rows = kernel.rebuild_money_inbox()?;
        Ok(u32::try_from(transaction_rows + commitment_rows + inbox_rows).unwrap_or(u32::MAX))
    })
}

#[tauri::command]
#[specta::specta]
pub fn rebuild_read_models(state: tauri::State<'_, AppState>) -> Result<u32, IpcError> {
    rebuild_read_models_impl(state.inner())
}

// ---- backup / restore (personal-cfo-ef3 / -au3 / -dvxm) ---------------------

/// Export an encrypted backup of the unlocked vault to `out_path` (a path the
/// user picked via the native Save dialog). The backup's `created_at` and
/// `backup_id` are stamped here; the password is scrubbed on return.
/// Escape one CSV field per RFC 4180: quote it when it carries a comma, quote,
/// or newline, doubling embedded quotes. Everything else passes through bare.
fn csv_field(raw: &str) -> String {
    if raw.contains(',') || raw.contains('"') || raw.contains('\n') || raw.contains('\r') {
        format!("\"{}\"", raw.replace('"', "\"\""))
    } else {
        raw.to_owned()
    }
}

/// Signed decimal for a money amount honoring the currency's exponent
/// (e.g. -1234 cents → "-12.34").
fn csv_amount(amount: finance_kernel::Money) -> String {
    let exponent = u32::from(amount.currency_exponent());
    let scale = 10_i64.pow(exponent);
    let minor = amount.minor_units();
    let sign = if minor < 0 { "-" } else { "" };
    let magnitude = minor.unsigned_abs();
    let scale_u = scale as u64;
    format!(
        "{sign}{}.{:0width$}",
        magnitude / scale_u,
        magnitude % scale_u,
        width = exponent as usize
    )
}

/// Export every transaction as plaintext CSV to `out_path` (personal-cfo-hbd8,
/// Launch AC#4 portability). Chronological (oldest first), deterministic ordering,
/// category/tag ids resolved to display names. This is a deliberate PLAINTEXT
/// export — the user chose the destination via the native save dialog; the command
/// sits in the destructive/exfiltration ACL capability (3fdd.6).
pub fn export_transactions_csv_impl(state: &AppState, out_path: String) -> Result<u32, IpcError> {
    with_kernel(state, |kernel| {
        let page = kernel.transaction_page(&finance_kernel::TransactionPageQuery {
            sort: finance_kernel::TransactionSortOrder::OldestFirst,
            limit: u32::MAX,
            ..Default::default()
        })?;
        let categories: std::collections::HashMap<String, String> = kernel
            .category_views()?
            .into_iter()
            .map(|c| (c.id.to_string(), c.name))
            .collect();
        let tags: std::collections::HashMap<String, String> = kernel
            .tag_views()?
            .into_iter()
            .map(|t| (t.id.to_string(), t.name))
            .collect();

        let mut csv =
            String::from("date,account,amount,currency,category,merchant,memo,tags,note\n");
        let count = u32::try_from(page.rows.len()).unwrap_or(u32::MAX);
        for row in page.rows {
            let date = row.occurred_at.format("%Y-%m-%d").to_string();
            let category = row
                .category_id
                .map(|id| id.to_string())
                .and_then(|id| categories.get(&id).cloned())
                .unwrap_or_default();
            let tag_names = row
                .tag_ids
                .iter()
                .filter_map(|id| tags.get(&id.to_string()).cloned())
                .collect::<Vec<_>>()
                .join("; ");
            let fields = [
                date,
                row.account_name,
                csv_amount(row.amount),
                row.amount.currency().code().to_owned(),
                category,
                row.counterparty.unwrap_or_default(),
                row.memo.unwrap_or_default(),
                tag_names,
                row.note.unwrap_or_default(),
            ];
            let line = fields
                .iter()
                .map(|f| csv_field(f))
                .collect::<Vec<_>>()
                .join(",");
            csv.push_str(&line);
            csv.push('\n');
        }
        std::fs::write(&out_path, csv)
            .map_err(|e| IpcError::Persistence(format!("writing the CSV export: {e}")))?;
        Ok(count)
    })
}

#[tauri::command]
#[specta::specta]
pub fn export_transactions_csv(
    state: tauri::State<'_, AppState>,
    out_path: String,
) -> Result<u32, IpcError> {
    export_transactions_csv_impl(state.inner(), out_path)
}

pub fn export_backup_impl(
    state: &AppState,
    password: String,
    out_path: String,
) -> Result<(), IpcError> {
    let password = Zeroizing::new(password);
    with_kernel(state, |kernel| {
        kernel.export_backup(
            password.as_bytes(),
            std::path::Path::new(&out_path),
            env!("CARGO_PKG_VERSION"),
            chrono::Utc::now().to_rfc3339(),
            Uuid::now_v7(),
        )?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn export_backup(
    state: tauri::State<'_, AppState>,
    password: String,
    out_path: String,
) -> Result<(), IpcError> {
    export_backup_impl(state.inner(), password, out_path)
}

/// Restore an encrypted backup from `package_path` into a fresh vault, leaving it
/// unlocked. Valid only when no vault exists yet (the fresh-clone case); the
/// engine refuses to clobber an existing vault.
pub fn restore_backup_impl(
    state: &AppState,
    package_path: String,
    password: String,
) -> Result<VaultStatusDto, IpcError> {
    let password = Zeroizing::new(password);
    let mut guard = state.lock_controller()?;
    guard.restore(std::path::Path::new(&package_path), password.as_bytes())?;
    vault_status_dto(&guard)
}

#[tauri::command]
#[specta::specta]
pub fn restore_backup(
    state: tauri::State<'_, AppState>,
    package_path: String,
    password: String,
) -> Result<VaultStatusDto, IpcError> {
    restore_backup_impl(state.inner(), package_path, password)
}

// ---- create_account --------------------------------------------------------

/// Create an account (optionally with an opening balance).
pub fn create_account_impl(
    state: &AppState,
    input: CreateAccountInput,
) -> Result<CreateAccountResult, IpcError> {
    with_kernel(state, |kernel| {
        let account = input.to_account()?;
        let account_id = account.id();

        let command = match input.opening_balance.as_ref() {
            Some(opening) => {
                let money: Money = opening.to_money()?;
                CreateAccount::with_opening_balance(account, money)
            }
            None => CreateAccount::new(account),
        };

        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        Ok(CreateAccountResult {
            account_id: account_id.to_string(),
            mutation: outcome.into(),
        })
    })
}

#[tauri::command]
#[specta::specta]
pub fn create_account(
    state: tauri::State<'_, AppState>,
    input: CreateAccountInput,
) -> Result<CreateAccountResult, IpcError> {
    create_account_impl(state.inner(), input)
}

// ---- record_transaction ----------------------------------------------------

/// Record a manual transaction (a signed money movement against an account). Returns
/// the id the transaction was stored under (personal-cfo-4d8.24.2.1) so the caller can
/// attach category/tags/notes to it inline.
///
/// The id is a deterministic function of the effective idempotency key, so it is the
/// *real* persisted id even on an idempotent replay: a retry (same key) is de-duplicated
/// to the original op and re-derives the same id the first call stored the row under —
/// never a phantom id. (`Outcome::Replayed` carries only the op sequence, so a
/// non-deterministic id could not be recovered on replay.)
pub fn record_transaction_impl(
    state: &AppState,
    input: RecordTransactionInput,
) -> Result<RecordTransactionResult, IpcError> {
    let key = resolve_idempotency_key(&input.idempotency_key);
    let transaction_id = TransactionId::from_uuid(Uuid::new_v5(
        &RECORD_TRANSACTION_ID_NAMESPACE,
        key.as_bytes(),
    ));
    with_kernel(state, |kernel| {
        let command = input.to_command(transaction_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta_with_key(key.clone()),
            command,
        ))?;
        Ok(RecordTransactionResult {
            transaction_id: transaction_id.as_uuid().to_string(),
            result: outcome.into(),
        })
    })
}

#[tauri::command]
#[specta::specta]
pub fn record_transaction(
    state: tauri::State<'_, AppState>,
    input: RecordTransactionInput,
) -> Result<RecordTransactionResult, IpcError> {
    record_transaction_impl(state.inner(), input)
}

/// Record a one-off transfer between two of the user's accounts (personal-cfo-npoe).
pub fn record_transfer_impl(
    state: &AppState,
    input: RecordTransferInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let command = input.to_command()?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn record_transfer(
    state: tauri::State<'_, AppState>,
    input: RecordTransferInput,
) -> Result<MutationResult, IpcError> {
    record_transfer_impl(state.inner(), input)
}

// ---- ingestion staging (personal-cfo-3bb, ADR 0008) ------------------------

/// Open an ingestion source batch. The batch id is minted here and returned so
/// the importer can attach records to it. Importers stage through this surface
/// and never write the ledger directly.
pub fn create_source_batch_impl(
    state: &AppState,
    input: CreateSourceBatchInput,
) -> Result<CreateSourceBatchResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = SourceBatchId::new();
        let command = CreateSourceBatch::new(
            id,
            input.source_type,
            input.source_name,
            input.file_fingerprint,
            input.parser_version,
        );
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        Ok(CreateSourceBatchResult {
            source_batch_id: id.to_string(),
            mutation: outcome.into(),
        })
    })
}

#[tauri::command]
#[specta::specta]
pub fn create_source_batch(
    state: tauri::State<'_, AppState>,
    input: CreateSourceBatchInput,
) -> Result<CreateSourceBatchResult, IpcError> {
    create_source_batch_impl(state.inner(), input)
}

/// Attach a parsed source record to a batch. Idempotent on `(batch, source_hash)`
/// — re-attaching the same content adds no new record (shred-after-parse keeps
/// the hash + normalized fields, never the raw bytes).
pub fn attach_source_record_impl(
    state: &AppState,
    input: AttachSourceRecordInput,
) -> Result<AttachSourceRecordResult, IpcError> {
    with_kernel(state, |kernel| {
        let batch_id = parse_source_batch_id(&input.source_batch_id)?;
        let id = SourceRecordId::new();
        let command = AttachSourceRecord::new(
            id,
            batch_id,
            input.external_id,
            input.source_hash,
            input.normalized_json,
            input.parse_confidence_bps,
        );
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        Ok(AttachSourceRecordResult {
            source_record_id: id.to_string(),
            mutation: outcome.into(),
        })
    })
}

#[tauri::command]
#[specta::specta]
pub fn attach_source_record(
    state: tauri::State<'_, AppState>,
    input: AttachSourceRecordInput,
) -> Result<AttachSourceRecordResult, IpcError> {
    attach_source_record_impl(state.inner(), input)
}

/// Advance a source batch's lifecycle status + progress counts (ADR 0008).
pub fn update_batch_state_impl(
    state: &AppState,
    input: UpdateBatchStateInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let batch_id = parse_source_batch_id(&input.source_batch_id)?;
        let command = UpdateBatchState::new(
            batch_id,
            input.status,
            input.staged_count,
            input.committed_count,
            input.skipped_count,
        );
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn update_batch_state(
    state: tauri::State<'_, AppState>,
    input: UpdateBatchStateInput,
) -> Result<MutationResult, IpcError> {
    update_batch_state_impl(state.inner(), input)
}

/// Import a file through the ingestion pipeline (personal-cfo-cu8): resolve the
/// importer plugin (by `plugin_id`, else auto-detect from the bytes), parse it in
/// the bounded host, then stage + dedupe + commit the clean rows into
/// `target_account`. Returns the batch outcome. The raw bytes are never persisted
/// unencrypted (ADR 0014 shred-after-parse).
pub fn import_batch_impl(
    state: &AppState,
    input: ImportBatchInput,
) -> Result<BatchResultDto, IpcError> {
    with_kernel(state, |kernel| {
        let target_account = parse_account_id(&input.target_account_id)?;
        let mut parser_input = ParserInput::new(input.data);
        if let Some(name) = input.filename {
            parser_input = parser_input.with_filename(name);
        }
        let default_currency = match input.default_currency.as_deref() {
            Some(code) => Some(parse_currency(code)?),
            None => None,
        };
        // A preset (personal-cfo-gvidg) is the BASE; the user's own
        // column_mapping/date_format/default_currency, when given, override
        // the matching field — the user can always correct a preset's guess.
        // No preset_id behaves exactly as before this field existed.
        let preset_hints = match input.preset_id.as_deref() {
            Some(id) => Some(
                preset_by_id(id)
                    .ok_or_else(|| IpcError::Validation(format!("no source preset {id:?}")))?
                    .hints(),
            ),
            None => None,
        };
        let hints = ParserHints {
            column_mapping: input
                .column_mapping
                .map(|m| m.into_mapping())
                .or_else(|| preset_hints.as_ref().and_then(|h| h.column_mapping.clone())),
            date_format: input
                .date_format
                .or_else(|| preset_hints.as_ref().and_then(|h| h.date_format.clone())),
            default_currency: default_currency
                .or_else(|| preset_hints.as_ref().and_then(|h| h.default_currency)),
            institution: preset_hints.as_ref().and_then(|h| h.institution.clone()),
        };
        let plugin = match input.plugin_id.as_deref() {
            Some(id) => plugin_by_id(id)
                .ok_or_else(|| IpcError::Validation(format!("no importer plugin {id:?}")))?,
            None => detect_best(&parser_input).ok_or_else(|| {
                IpcError::Validation("no importer recognized this file".to_owned())
            })?,
        };
        let result = kernel.ingest_batch(
            plugin,
            parser_input,
            &hints,
            target_account,
            &ParserLimits::default(),
            &user_meta(&input.idempotency_key),
        )?;
        Ok(BatchResultDto::from(result))
    })
}

#[tauri::command]
#[specta::specta]
pub fn import_batch(
    state: tauri::State<'_, AppState>,
    input: ImportBatchInput,
) -> Result<BatchResultDto, IpcError> {
    import_batch_impl(state.inner(), input)
}

/// The source column headers of a file, for the import column-mapping UI
/// (personal-cfo-4d8.24.1.2). Resolves the importer plugin (by `plugin_id`, else
/// auto-detect from the bytes) and returns its `preview_columns` — the exact header
/// strings a `column_mapping` is matched against. Empty for formats without mappable
/// columns (e.g. OFX) or when no importer recognizes the file. Nothing is persisted.
pub fn import_preview_columns_impl(
    state: &AppState,
    data: Vec<u8>,
    filename: Option<String>,
    plugin_id: Option<String>,
) -> Result<Vec<String>, IpcError> {
    with_kernel(state, |_kernel| {
        let mut parser_input = ParserInput::new(data);
        if let Some(name) = filename {
            parser_input = parser_input.with_filename(name);
        }
        let plugin = match plugin_id.as_deref() {
            Some(id) => plugin_by_id(id)
                .ok_or_else(|| IpcError::Validation(format!("no importer plugin {id:?}")))?,
            None => match detect_best(&parser_input) {
                Some(plugin) => plugin,
                None => return Ok(Vec::new()),
            },
        };
        Ok(plugin.preview_columns(&parser_input))
    })
}

#[tauri::command]
#[specta::specta]
pub fn import_preview_columns(
    state: tauri::State<'_, AppState>,
    data: Vec<u8>,
    filename: Option<String>,
    plugin_id: Option<String>,
) -> Result<Vec<String>, IpcError> {
    import_preview_columns_impl(state.inner(), data, filename, plugin_id)
}

/// Every registered source-app preset (personal-cfo-gvidg), for the "Import
/// from <app>" picker. No vault access, no state needed — the registry is
/// compile-time and process-global — but takes `&AppState` for the same
/// reason every other `*_impl` does: a uniform signature integration tests
/// can call without constructing a `tauri::State`.
pub fn list_source_presets_impl(_state: &AppState) -> Vec<SourcePresetDto> {
    all_presets()
        .map(|preset| {
            let hints = preset.hints();
            SourcePresetDto {
                id: preset.id().to_owned(),
                display_name: preset.display_name().to_owned(),
                source_app_url: preset.source_app_url().to_owned(),
                column_mapping: ColumnMappingDto::from_mapping(
                    &hints.column_mapping.unwrap_or_default(),
                ),
                help_slug: preset.help_slug().to_owned(),
                help_published: preset.help_published(),
            }
        })
        .collect()
}

#[tauri::command]
#[specta::specta]
pub fn list_source_presets(state: tauri::State<'_, AppState>) -> Vec<SourcePresetDto> {
    list_source_presets_impl(state.inner())
}

// ---- attachments (ADR 0023) ------------------------------------------------

/// Attach a document (`data` = the raw file bytes) to a transaction: encrypt and
/// store it, then link it to the transaction. Returns the new attachment's
/// metadata. The plaintext bytes are encrypted in the worker and never written
/// outside the vault.
pub fn attach_document_impl(
    state: &AppState,
    transaction_id: String,
    filename: Option<String>,
    mime_type: Option<String>,
    data: Vec<u8>,
) -> Result<AttachmentDto, IpcError> {
    with_kernel(state, |kernel| {
        let txn = parse_transaction_id(&transaction_id)?;
        let meta = kernel.attach_document(txn, &data, mime_type.as_deref(), filename.as_deref())?;
        Ok(meta.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn attach_document(
    state: tauri::State<'_, AppState>,
    transaction_id: String,
    filename: Option<String>,
    mime_type: Option<String>,
    data: Vec<u8>,
) -> Result<AttachmentDto, IpcError> {
    attach_document_impl(state.inner(), transaction_id, filename, mime_type, data)
}

/// The documents attached to a transaction (metadata only — no bytes).
pub fn transaction_attachments_impl(
    state: &AppState,
    transaction_id: String,
) -> Result<Vec<AttachmentDto>, IpcError> {
    with_kernel(state, |kernel| {
        let txn = parse_transaction_id(&transaction_id)?;
        Ok(kernel
            .transaction_attachments(txn)?
            .into_iter()
            .map(AttachmentDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn transaction_attachments(
    state: tauri::State<'_, AppState>,
    transaction_id: String,
) -> Result<Vec<AttachmentDto>, IpcError> {
    transaction_attachments_impl(state.inner(), transaction_id)
}

/// The raw imported source fields behind a transaction (ADR 0045 §2): every column
/// the importer captured, resolved via the provenance link. `None` for a manually-
/// entered transaction (no import).
pub fn imported_transaction_fields_impl(
    state: &AppState,
    transaction_id: String,
) -> Result<Option<ImportedTransactionFieldsDto>, IpcError> {
    with_kernel(state, |kernel| {
        let txn = parse_transaction_id(&transaction_id)?;
        Ok(kernel
            .imported_transaction_fields(txn)?
            .map(ImportedTransactionFieldsDto::from))
    })
}

#[tauri::command]
#[specta::specta]
pub fn imported_transaction_fields(
    state: tauri::State<'_, AppState>,
    transaction_id: String,
) -> Result<Option<ImportedTransactionFieldsDto>, IpcError> {
    imported_transaction_fields_impl(state.inner(), transaction_id)
}

/// Detach a document from a transaction; removing the last link crypto-shreds
/// the attachment (ADR 0023).
pub fn remove_attachment_impl(
    state: &AppState,
    attachment_id: String,
    transaction_id: String,
) -> Result<(), IpcError> {
    with_kernel(state, |kernel| {
        let att = parse_attachment_id(&attachment_id)?;
        let txn = parse_transaction_id(&transaction_id)?;
        kernel.remove_attachment(att, txn)?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn remove_attachment(
    state: tauri::State<'_, AppState>,
    attachment_id: String,
    transaction_id: String,
) -> Result<(), IpcError> {
    remove_attachment_impl(state.inner(), attachment_id, transaction_id)
}

// ---- void_transaction ------------------------------------------------------

/// Delete a transaction by voiding it (personal-cfo-4d8.11, ADR 0007 §9): the kernel
/// posts a reversing entry and hides the original + reversal from every view, so
/// balances + forecast update while the ledger stays append-only. Idempotency-keyed.
pub fn void_transaction_impl(
    state: &AppState,
    transaction_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let txn = parse_transaction_id(&transaction_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            VoidTransaction::new(txn),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn void_transaction(
    state: tauri::State<'_, AppState>,
    transaction_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    void_transaction_impl(state.inner(), transaction_id, idempotency_key)
}

// ---- mark_reviewed ---------------------------------------------------------

/// Mark a transaction reviewed or unreviewed (personal-cfo-4d8.7, ADR 0032 §2):
/// records the user's explicit override. Idempotency-keyed.
pub fn mark_reviewed_impl(
    state: &AppState,
    transaction_id: String,
    reviewed: bool,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let txn = parse_transaction_id(&transaction_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            MarkReviewed::new(txn, reviewed),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn mark_reviewed(
    state: tauri::State<'_, AppState>,
    transaction_id: String,
    reviewed: bool,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    mark_reviewed_impl(state.inner(), transaction_id, reviewed, idempotency_key)
}

// ---- tags + notes (ADR 0033) -----------------------------------------------

/// Every tag, for the picker (personal-cfo-2ryf).
pub fn tag_list_impl(state: &AppState) -> Result<Vec<TagViewDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .tag_views()?
            .into_iter()
            .map(TagViewDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn tag_list(state: tauri::State<'_, AppState>) -> Result<Vec<TagViewDto>, IpcError> {
    tag_list_impl(state.inner())
}

/// Create a user-defined tag (ADR 0033); the id is minted here and returned.
pub fn create_tag_impl(
    state: &AppState,
    name: String,
    color: Option<String>,
    idempotency_key: String,
) -> Result<CreateTagResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = TagId::new();
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            CreateTag::new(id, name, color),
        ))?;
        Ok(CreateTagResult {
            tag_id: id.to_string(),
            mutation: outcome.into(),
        })
    })
}

#[tauri::command]
#[specta::specta]
pub fn create_tag(
    state: tauri::State<'_, AppState>,
    name: String,
    color: Option<String>,
    idempotency_key: String,
) -> Result<CreateTagResult, IpcError> {
    create_tag_impl(state.inner(), name, color, idempotency_key)
}

/// Replace a transaction's tag set (ADR 0033).
pub fn set_tags_impl(
    state: &AppState,
    transaction_id: String,
    tag_ids: Vec<String>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let txn = parse_transaction_id(&transaction_id)?;
        let tags = tag_ids
            .iter()
            .map(|t| parse_tag_id(t))
            .collect::<Result<Vec<_>, _>>()?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            SetTags::new(txn, tags),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_tags(
    state: tauri::State<'_, AppState>,
    transaction_id: String,
    tag_ids: Vec<String>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    set_tags_impl(state.inner(), transaction_id, tag_ids, idempotency_key)
}

/// Set or clear a transaction's free-text note (ADR 0033 §3).
pub fn set_note_impl(
    state: &AppState,
    transaction_id: String,
    note: Option<String>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let txn = parse_transaction_id(&transaction_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            SetNote::new(txn, note),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_note(
    state: tauri::State<'_, AppState>,
    transaction_id: String,
    note: Option<String>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    set_note_impl(state.inner(), transaction_id, note, idempotency_key)
}

/// Replace a transaction's split set (ADR 0034, personal-cfo-e7i).
pub fn set_splits_impl(
    state: &AppState,
    transaction_id: String,
    lines: Vec<SplitLineInputDto>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let txn = parse_transaction_id(&transaction_id)?;
        let parsed = lines
            .into_iter()
            .map(SplitLineInputDto::into_input)
            .collect::<Result<Vec<_>, _>>()?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            SetSplits::new(txn, parsed),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_splits(
    state: tauri::State<'_, AppState>,
    transaction_id: String,
    lines: Vec<SplitLineInputDto>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    set_splits_impl(state.inner(), transaction_id, lines, idempotency_key)
}

/// A transaction's split lines, ordered (personal-cfo-kr9).
pub fn transaction_splits_impl(
    state: &AppState,
    transaction_id: String,
) -> Result<Vec<SplitLineDto>, IpcError> {
    with_kernel(state, |kernel| {
        let txn = parse_transaction_id(&transaction_id)?;
        Ok(kernel
            .transaction_splits(txn)?
            .into_iter()
            .map(SplitLineDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn transaction_splits(
    state: tauri::State<'_, AppState>,
    transaction_id: String,
) -> Result<Vec<SplitLineDto>, IpcError> {
    transaction_splits_impl(state.inner(), transaction_id)
}

/// The committed counterpart(s) a flagged staged transaction may duplicate (ADR 0032 §4,
/// personal-cfo-4d8.20) — backs the duplicate Review panel's "in your ledger" column.
pub fn duplicate_candidates_impl(
    state: &AppState,
    staged_transaction_id: String,
) -> Result<Vec<TransactionRowDto>, IpcError> {
    with_kernel(state, |kernel| {
        let staged = parse_staged_transaction_id(&staged_transaction_id)?;
        Ok(kernel
            .duplicate_candidates(staged)?
            .into_iter()
            .map(TransactionRowDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn duplicate_candidates(
    state: tauri::State<'_, AppState>,
    staged_transaction_id: String,
) -> Result<Vec<TransactionRowDto>, IpcError> {
    duplicate_candidates_impl(state.inner(), staged_transaction_id)
}

// ---- update_account --------------------------------------------------------

/// Rename an account.
pub fn update_account_impl(
    state: &AppState,
    input: UpdateAccountInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_account_id(&input.account_id)?;
        let command = UpdateAccount::new(id, input.name.clone());
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn update_account(
    state: tauri::State<'_, AppState>,
    input: UpdateAccountInput,
) -> Result<MutationResult, IpcError> {
    update_account_impl(state.inner(), input)
}

// ---- set_account_subtype ---------------------------------------------------

/// Set (or clear) an account's subtype (ADR 0028, personal-cfo-9dgg). A `None`
/// (or empty) `subtype` clears it; the kernel rejects a subtype that does not
/// belong to the account's cashflow role.
pub fn set_account_subtype_impl(
    state: &AppState,
    account_id: String,
    subtype: Option<String>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_account_id(&account_id)?;
        let subtype = parse_subtype(subtype.as_deref())?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            SetAccountSubtype::new(id, subtype),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_account_subtype(
    state: tauri::State<'_, AppState>,
    account_id: String,
    subtype: Option<String>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    set_account_subtype_impl(state.inner(), account_id, subtype, idempotency_key)
}

// ---- set_account_note (ADR 0044, personal-cfo-4d8.22.4) --------------------

/// Set (or clear, with `None`) an account's free-text note.
pub fn set_account_note_impl(
    state: &AppState,
    account_id: String,
    note: Option<String>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_account_id(&account_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            SetAccountNote::new(id, note),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_account_note(
    state: tauri::State<'_, AppState>,
    account_id: String,
    note: Option<String>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    set_account_note_impl(state.inner(), account_id, note, idempotency_key)
}

// ---- set_account_link (ADR 0044 §5, personal-cfo-4d8.22.3) -----------------

/// Link a real asset to the liability that finances it, or clear it (`None` liability).
/// The worker validates the asset/liability roles.
pub fn set_account_link_impl(
    state: &AppState,
    asset_account_id: String,
    liability_account_id: Option<String>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let asset_id = parse_account_id(&asset_account_id)?;
        let liability_id = match liability_account_id.as_deref() {
            Some(s) if !s.trim().is_empty() => Some(parse_account_id(s)?),
            _ => None,
        };
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            SetAccountLink::new(asset_id, liability_id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_account_link(
    state: tauri::State<'_, AppState>,
    asset_account_id: String,
    liability_account_id: Option<String>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    set_account_link_impl(
        state.inner(),
        asset_account_id,
        liability_account_id,
        idempotency_key,
    )
}

// ---- set_debt_terms / debt_terms (ADR 0035, personal-cfo-6wk.2) ------------

/// Upsert a liability account's debt terms (ADR 0035 §5). The kernel/worker reject a
/// non-liability target or a non-liquid paying source.
pub fn set_debt_terms_impl(
    state: &AppState,
    input: SetDebtTermsInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_account_id(&input.account_id)?;
        let terms = input.to_core()?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            SetDebtTerms::new(id, terms),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_debt_terms(
    state: tauri::State<'_, AppState>,
    input: SetDebtTermsInput,
) -> Result<MutationResult, IpcError> {
    set_debt_terms_impl(state.inner(), input)
}

/// Record — or clear — a card statement's REAL balance for one cycle (feedback
/// 2026-07-03). The forecast prefers it over the estimate and carries forward from it.
pub fn set_card_statement_balance_impl(
    state: &AppState,
    input: SetCardStatementBalanceInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_account_id(&input.account_id)?;
        let cycle_close =
            chrono::NaiveDate::parse_from_str(input.cycle_close.trim(), "%Y-%m-%d")
                .map_err(|_| IpcError::Validation("cycle_close must be YYYY-MM-DD".to_owned()))?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            SetCardStatementBalance::new(id, cycle_close, input.statement_balance_minor),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_card_statement_balance(
    state: tauri::State<'_, AppState>,
    input: SetCardStatementBalanceInput,
) -> Result<MutationResult, IpcError> {
    set_card_statement_balance_impl(state.inner(), input)
}

/// Read a liability account's debt terms (ADR 0035 §5), or `None` if unset.
pub fn debt_terms_impl(
    state: &AppState,
    account_id: String,
) -> Result<Option<DebtTermsDto>, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_account_id(&account_id)?;
        Ok(kernel.debt_terms(id)?.map(DebtTermsDto::from))
    })
}

#[tauri::command]
#[specta::specta]
pub fn debt_terms(
    state: tauri::State<'_, AppState>,
    account_id: String,
) -> Result<Option<DebtTermsDto>, IpcError> {
    debt_terms_impl(state.inner(), account_id)
}

/// Debt terms for every account that has them, optionally scoped to an account set
/// (personal-cfo-4d17).
///
/// The Debt page needs terms for every debt in scope at once; the single-account read would
/// mean one round-trip per debt and N loading states to reconcile. Accounts WITHOUT terms
/// are omitted rather than returned empty — "no rate recorded" and "no interest" are
/// different facts, and a row of nulls would erase the difference.
pub fn debt_terms_list_impl(
    state: &AppState,
    account_ids: &[String],
) -> Result<Vec<DebtTermsDto>, IpcError> {
    let scope = account_ids
        .iter()
        .map(|raw| raw.trim())
        .filter(|raw| !raw.is_empty())
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()?;
    with_kernel(state, |kernel| {
        Ok(kernel
            .debt_terms_list(&scope)?
            .into_iter()
            .map(DebtTermsDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn debt_terms_list(
    state: tauri::State<'_, AppState>,
    // Empty means every debt account with terms.
    account_ids: Vec<String>,
) -> Result<Vec<DebtTermsDto>, IpcError> {
    debt_terms_list_impl(state.inner(), &account_ids)
}

// ---- archive_account -------------------------------------------------------

/// Archive (soft-hide) an account. Non-destructive: postings are preserved.
pub fn archive_account_impl(
    state: &AppState,
    account_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_account_id(&account_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            ArchiveAccount::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn archive_account(
    state: tauri::State<'_, AppState>,
    account_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    archive_account_impl(state.inner(), account_id, idempotency_key)
}

// ---- reinstate_account -----------------------------------------------------

/// Reinstate a previously archived account.
pub fn reinstate_account_impl(
    state: &AppState,
    account_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_account_id(&account_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            ReinstateAccount::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn reinstate_account(
    state: tauri::State<'_, AppState>,
    account_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    reinstate_account_impl(state.inner(), account_id, idempotency_key)
}

// ---- reads -----------------------------------------------------------------

/// Number of accounts in the vault.
pub fn account_count_impl(state: &AppState) -> Result<u32, IpcError> {
    with_kernel(state, |kernel| {
        let count = kernel.account_count()?;
        // Counts in a personal vault fit a u32 comfortably; clamp defensively
        // rather than overflow the wire type.
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    })
}

#[tauri::command]
#[specta::specta]
pub fn account_count(state: tauri::State<'_, AppState>) -> Result<u32, IpcError> {
    account_count_impl(state.inner())
}

/// The balance of an account, or `None` if it does not exist.
pub fn account_balance_impl(
    state: &AppState,
    account_id: String,
) -> Result<Option<MoneyDto>, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_account_id(&account_id)?;
        Ok(kernel.account_balance(id)?.map(MoneyDto::from))
    })
}

#[tauri::command]
#[specta::specta]
pub fn account_balance(
    state: tauri::State<'_, AppState>,
    account_id: String,
) -> Result<Option<MoneyDto>, IpcError> {
    account_balance_impl(state.inner(), account_id)
}

/// The read-model view of an account, or `None` if it does not exist.
pub fn account_view_impl(
    state: &AppState,
    account_id: String,
) -> Result<Option<AccountViewDto>, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_account_id(&account_id)?;
        Ok(kernel.account_view(id)?.map(AccountViewDto::from))
    })
}

#[tauri::command]
#[specta::specta]
pub fn account_view(
    state: tauri::State<'_, AppState>,
    account_id: String,
) -> Result<Option<AccountViewDto>, IpcError> {
    account_view_impl(state.inner(), account_id)
}

/// Every account in the vault, ordered by name. Backs the accounts list UI.
pub fn account_list_impl(state: &AppState) -> Result<Vec<AccountViewDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .account_views()?
            .into_iter()
            .map(AccountViewDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn account_list(state: tauri::State<'_, AppState>) -> Result<Vec<AccountViewDto>, IpcError> {
    account_list_impl(state.inner())
}

// ---- cash_tiers ------------------------------------------------------------

/// The type-based cash-tier rollups (ADR 0028, personal-cfo-9dgg): spendable /
/// reserve / net cash, derived from liquid accounts' subtypes. Backs the Accounts
/// view's cash-tier summary.
pub fn cash_tiers_impl(state: &AppState) -> Result<CashTiersDto, IpcError> {
    with_kernel(state, |kernel| Ok(kernel.cash_tiers()?.into()))
}

#[tauri::command]
#[specta::specta]
pub fn cash_tiers(state: tauri::State<'_, AppState>) -> Result<CashTiersDto, IpcError> {
    cash_tiers_impl(state.inner())
}

// ---- cash availability (ADR 0029, personal-cfo-fqbm) ------------------------

/// The household cash-availability snapshot: ledger / available / pending /
/// committed / headroom per liquid account, the net rollup, and the minimum-cash-
/// floor status.
pub fn cash_availability_impl(state: &AppState) -> Result<CashAvailabilityDto, IpcError> {
    with_kernel(state, |kernel| Ok(kernel.cash_availability()?.into()))
}

#[tauri::command]
#[specta::specta]
pub fn cash_availability(
    state: tauri::State<'_, AppState>,
) -> Result<CashAvailabilityDto, IpcError> {
    cash_availability_impl(state.inner())
}

/// The R1 Forecast Readiness score (ADR 0026 §13, personal-cfo-6vj9): a 0–100
/// data-maturity indicator (coverage + balance freshness + explained ratio) with a
/// per-factor breakdown the dashboard surfaces.
pub fn forecast_readiness_impl(state: &AppState) -> Result<ForecastReadinessDto, IpcError> {
    with_kernel(state, |kernel| Ok(kernel.forecast_readiness()?.into()))
}

#[tauri::command]
#[specta::specta]
pub fn forecast_readiness(
    state: tauri::State<'_, AppState>,
) -> Result<ForecastReadinessDto, IpcError> {
    forecast_readiness_impl(state.inner())
}

/// Forecast capabilities that have self-activated but whose one-time unlock notice the user
/// has not yet acknowledged (ADR 0026 §10, personal-cfo-egon). The frontend shows one
/// dismissible notice per entry and calls `acknowledge_capability` on dismiss.
pub fn pending_capability_unlocks_impl(
    state: &AppState,
) -> Result<Vec<CapabilityUnlockDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .pending_capability_unlocks()?
            .into_iter()
            .map(CapabilityUnlockDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn pending_capability_unlocks(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CapabilityUnlockDto>, IpcError> {
    pending_capability_unlocks_impl(state.inner())
}

/// Record acknowledgement of a capability-unlock notice so it fires exactly once
/// (ADR 0026 §10). `key` is the `CapabilityUnlockDto.key` the user dismissed.
pub fn acknowledge_capability_impl(state: &AppState, key: &str) -> Result<(), IpcError> {
    with_kernel(state, |kernel| {
        kernel.acknowledge_capability(&user_meta(""), key)?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn acknowledge_capability(
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<(), IpcError> {
    acknowledge_capability_impl(state.inner(), &key)
}

/// Apply merchant-memory auto-categorization (ADR 0030 addendum, personal-cfo-7yh0):
/// learn merchant→category from the user's manual categorizations and fill uncategorized
/// transactions of the same merchant. Returns the number newly categorized.
pub fn apply_merchant_memory_impl(state: &AppState) -> Result<u32, IpcError> {
    with_kernel(state, |kernel| Ok(kernel.apply_merchant_memory()?))
}

#[tauri::command]
#[specta::specta]
pub fn apply_merchant_memory(state: tauri::State<'_, AppState>) -> Result<u32, IpcError> {
    apply_merchant_memory_impl(state.inner())
}

/// Whether merchant memory auto-applies after an import (ADR 0030 addendum,
/// personal-cfo-5n4.2). Defaults to `true` when never set.
pub fn auto_categorize_on_import_impl(state: &AppState) -> Result<bool, IpcError> {
    with_kernel(state, |kernel| Ok(kernel.auto_categorize_on_import()?))
}

#[tauri::command]
#[specta::specta]
pub fn auto_categorize_on_import(state: tauri::State<'_, AppState>) -> Result<bool, IpcError> {
    auto_categorize_on_import_impl(state.inner())
}

/// Set whether merchant memory auto-applies after an import (ADR 0030 addendum,
/// personal-cfo-5n4.2).
pub fn set_auto_categorize_on_import_impl(state: &AppState, enabled: bool) -> Result<(), IpcError> {
    with_kernel(state, |kernel| {
        kernel.set_auto_categorize_on_import(enabled)?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_auto_categorize_on_import(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), IpcError> {
    set_auto_categorize_on_import_impl(state.inner(), enabled)
}

/// The Future Cash chart's stored series-selection preference (opaque JSON), or
/// `null` when never set (personal-cfo-4d8.25.26).
pub fn future_cash_series_selection_impl(state: &AppState) -> Result<Option<String>, IpcError> {
    with_kernel(state, |kernel| Ok(kernel.future_cash_series_selection()?))
}

#[tauri::command]
#[specta::specta]
pub fn future_cash_series_selection(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, IpcError> {
    future_cash_series_selection_impl(state.inner())
}

/// Persist the Future Cash chart's series-selection preference (opaque JSON).
pub fn set_future_cash_series_selection_impl(
    state: &AppState,
    selection: String,
) -> Result<(), IpcError> {
    with_kernel(state, |kernel| {
        kernel.set_future_cash_series_selection(&selection)?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_future_cash_series_selection(
    state: tauri::State<'_, AppState>,
    selection: String,
) -> Result<(), IpcError> {
    set_future_cash_series_selection_impl(state.inner(), selection)
}

/// Set the household minimum-cash floor (the buffer to keep). Stored as minor units
/// in the `settings` table; interpreted in the forecast currency. The `floor`'s
/// currency is informational — only its `minor_units` is persisted.
pub fn set_minimum_cash_floor_impl(state: &AppState, floor: MoneyDto) -> Result<(), IpcError> {
    if floor.minor_units < 0 {
        return Err(IpcError::Validation(
            "minimum cash floor must not be negative".to_owned(),
        ));
    }
    with_kernel(state, |kernel| {
        kernel.set_setting(MINIMUM_CASH_FLOOR_KEY, &floor.minor_units.to_string())?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_minimum_cash_floor(
    state: tauri::State<'_, AppState>,
    floor: MoneyDto,
) -> Result<(), IpcError> {
    set_minimum_cash_floor_impl(state.inner(), floor)
}

/// The household liquid-cash comfort band — lower edge (the minimum-cash floor) + optional upper
/// edge (ADR 0018 addendum 915.1, personal-cfo-3v6d).
pub fn comfort_band_impl(state: &AppState) -> Result<ComfortBandDto, IpcError> {
    with_kernel(state, |kernel| Ok(kernel.comfort_band()?.into()))
}

#[tauri::command]
#[specta::specta]
pub fn comfort_band(state: tauri::State<'_, AppState>) -> Result<ComfortBandDto, IpcError> {
    comfort_band_impl(state.inner())
}

/// The far-horizon window the drift signal projects over — a band drift is inherently a distant
/// crossing, independent of the chart's zoom (ADR 0018 timing addendum, personal-cfo-5ie.8).
const DRIFT_HORIZON_DAYS: u32 = 180;

/// The current comfort-band drift signal — the descriptive "why you're heading below the band"
/// (personal-cfo-5ie.8). `None` when there's no far-horizon spending-driven crossing.
pub fn band_drift_signal_impl(state: &AppState) -> Result<Option<BandDriftSignalDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .band_drift(chrono::Utc::now(), DRIFT_HORIZON_DAYS)?
            .map(BandDriftSignalDto::from))
    })
}

#[tauri::command]
#[specta::specta]
pub fn band_drift_signal(
    state: tauri::State<'_, AppState>,
) -> Result<Option<BandDriftSignalDto>, IpcError> {
    band_drift_signal_impl(state.inner())
}

/// Set (`Some`) or clear (`None`) the comfort band's UPPER edge. The LOWER edge stays the
/// minimum-cash floor (set via `set_minimum_cash_floor`), unchanged (ADR 0018 addendum 915.1).
///
/// The two edges are stored as independent settings; the band-range invariant (`upper >= lower`)
/// is enforced by the settings UI (personal-cfo-3v6d PR2), which edits both together, rather than
/// here — so the shipped floor command is left untouched per 915.1. Clearing (`None`) writes a
/// blank value, which `comfort_band()` reads back as "no upper edge" (same parse-to-`None` the
/// floor read uses); this key has a single reader.
pub fn set_comfort_band_upper_impl(
    state: &AppState,
    upper: Option<MoneyDto>,
) -> Result<(), IpcError> {
    if upper.as_ref().is_some_and(|u| u.minor_units < 0) {
        return Err(IpcError::Validation(
            "comfort band upper edge must not be negative".to_owned(),
        ));
    }
    with_kernel(state, |kernel| {
        let value = upper.map(|u| u.minor_units.to_string()).unwrap_or_default();
        kernel.set_setting(COMFORT_BAND_UPPER_KEY, &value)?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_comfort_band_upper(
    state: tauri::State<'_, AppState>,
    upper: Option<MoneyDto>,
) -> Result<(), IpcError> {
    set_comfort_band_upper_impl(state.inner(), upper)
}

/// The household's current IANA timezone (`personal-cfo-q329`) — the value ADR 0021 §1
/// resolves every calendar boundary against. Reuses `vault_metadata` rather than adding a
/// new db-worker/kernel accessor for a single already-exposed field.
pub fn household_timezone_impl(state: &AppState) -> Result<String, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel.vault_metadata()?.household_timezone)
    })
}

#[tauri::command]
#[specta::specta]
pub fn household_timezone(state: tauri::State<'_, AppState>) -> Result<String, IpcError> {
    household_timezone_impl(state.inner())
}

/// Set the household's IANA timezone (`personal-cfo-q329`). Validated against
/// `chrono_tz::Tz` inside the kernel/db-worker layer (`DbError::InvalidCommand` on an
/// unrecognized name), not re-validated here — one source of truth for "what counts as a
/// valid timezone," the same rule `read_household_tz` and its callers already enforce on
/// read.
pub fn set_household_timezone_impl(state: &AppState, tz: String) -> Result<(), IpcError> {
    with_kernel(state, |kernel| Ok(kernel.set_household_timezone(&tz)?))
}

#[tauri::command]
#[specta::specta]
pub fn set_household_timezone(
    state: tauri::State<'_, AppState>,
    tz: String,
) -> Result<(), IpcError> {
    set_household_timezone_impl(state.inner(), tz)
}

/// Parse an ORDERED scenario selection (personal-cfo-4d8.27.6.4, ADR 0059 §1).
///
/// Order is the precedence, so the sequence is preserved exactly as the frontend sent it.
/// Blank entries are dropped rather than rejected, matching every other facet's handling of
/// the frontend's ""-means-unset state.
fn parse_scenario_selection(raw: &[String]) -> Result<Vec<uuid::Uuid>, IpcError> {
    raw.iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(parse_uuid)
        .collect()
}

/// The deterministic Future Cash forecast over the next `horizon_days` — the
/// liquid-cash opening balance folded with the income + recurring-obligation
/// event stream (personal-cfo-164u). Backs the dashboard's Future Cash widgets.
///
/// `scenario_ids` is the ORDERED overlay selection: empty is the base forecast; each entry
/// layers that scenario's scoped events, with a later entry winning a conflict with an
/// earlier one (ADR 0059 §1). The compare-vs-base view calls this twice — base and
/// selection (personal-cfo-6zep).
pub fn future_cash_forecast_impl(
    state: &AppState,
    horizon_days: u32,
    scenario_ids: Vec<String>,
) -> Result<ForecastViewDto, IpcError> {
    let scenarios = parse_scenario_selection(&scenario_ids)?;
    with_kernel(state, |kernel| {
        Ok(ForecastViewDto::from(
            kernel.future_cash_forecast(horizon_days, &scenarios)?,
        ))
    })
}

#[tauri::command]
#[specta::specta]
pub fn future_cash_forecast(
    state: tauri::State<'_, AppState>,
    horizon_days: u32,
    // The ORDERED scenario selection; empty is the base forecast. Order is the precedence
    // (ADR 0059 §1).
    scenario_ids: Vec<String>,
) -> Result<ForecastViewDto, IpcError> {
    future_cash_forecast_impl(state.inner(), horizon_days, scenario_ids)
}

// ---- card_statement_forecast (personal-cfo-4lhm) ---------------------------

/// The per-card credit-card statement + payment forecast (ADR 0039 §2): each card's
/// upcoming cycles with the projected statement balance, minimum due, full-pay amount,
/// and the payment its repayment philosophy selects. Backs the credit-card view.
pub fn card_statement_forecast_impl(
    state: &AppState,
) -> Result<Vec<CardStatementForecastDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .card_statement_forecast()?
            .into_iter()
            .map(CardStatementForecastDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn card_statement_forecast(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CardStatementForecastDto>, IpcError> {
    card_statement_forecast_impl(state.inner())
}

// ---- card_statement_history (personal-cfo-4d8.25.4) ------------------------

/// The past billing-cycle windows for one card with derived-from-imports charge totals and
/// any user-recorded actual statements (ADR 0039 addendum 2026-07-10 §2) — the statement
/// history capture surface. Newest first; empty for an account with no payment boundary.
pub fn card_statement_history_impl(
    state: &AppState,
    account_id: String,
) -> Result<Vec<CardStatementHistoryDto>, IpcError> {
    let id = parse_account_id(&account_id)?;
    with_kernel(state, |kernel| {
        Ok(kernel
            .card_statement_history(id)?
            .into_iter()
            .map(CardStatementHistoryDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn card_statement_history(
    state: tauri::State<'_, AppState>,
    account_id: String,
) -> Result<Vec<CardStatementHistoryDto>, IpcError> {
    card_statement_history_impl(state.inner(), account_id)
}

// ---- debt_payoff_comparison (personal-cfo-od07) ----------------------------

/// Compare debt-paydown strategies (minimum-only / snowball / avalanche) for the current
/// debts at `extra_budget_minor` extra per month: each plan's debt-free month + total
/// interest, for the Debt sub-view's descriptive compare (ADR 0018).
pub fn debt_payoff_comparison_impl(
    state: &AppState,
    extra_budget_minor: i64,
    account_ids: &[String],
) -> Result<Vec<DebtPayoffPlanDto>, IpcError> {
    // Blank entries are dropped rather than rejected, matching every other facet's handling
    // of the frontend's ""-means-unset state.
    let scope = account_ids
        .iter()
        .map(|raw| raw.trim())
        .filter(|raw| !raw.is_empty())
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()?;
    with_kernel(state, |kernel| {
        Ok(kernel
            .debt_payoff_comparison(extra_budget_minor, &scope)?
            .into_iter()
            .map(DebtPayoffPlanDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn debt_payoff_comparison(
    state: tauri::State<'_, AppState>,
    // A non-negative extra budget in minor units. `u32` (up to ~$42.9M/mo) keeps the wire type a
    // plain TS `number`; the engine works in `i64`.
    extra_budget_minor: u32,
    // Restrict the comparison to these debt accounts; empty means every debt
    // (personal-cfo-4d8.27.9.7).
    account_ids: Vec<String>,
) -> Result<Vec<DebtPayoffPlanDto>, IpcError> {
    debt_payoff_comparison_impl(state.inner(), i64::from(extra_budget_minor), &account_ids)
}

// ---- loan_double_count_warnings (personal-cfo-6wk.11) ----------------------

/// Suspected loan double-counts — a loan tracked as both a `loan_liability` account with payment
/// terms and an active recurring `loan_payment` bill (which double-counts its payment in the
/// liquid forecast). A descriptive warning for the Debt sub-view (ADR 0018); never auto-removes.
pub fn loan_double_count_warnings_impl(
    state: &AppState,
) -> Result<Vec<LoanDoubleCountWarningDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .loan_double_count_warnings()?
            .into_iter()
            .map(LoanDoubleCountWarningDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn loan_double_count_warnings(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<LoanDoubleCountWarningDto>, IpcError> {
    loan_double_count_warnings_impl(state.inner())
}

// ---- recurring_candidates (personal-cfo-98ql) -----------------------------

/// Recurring inbound deposits not yet modeled as income sources
/// (personal-cfo-gmnk): the bill detector pointed at the inflow side.
pub fn income_candidates_impl(state: &AppState) -> Result<Vec<RecurringCandidateDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .income_candidates()?
            .into_iter()
            .map(RecurringCandidateDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn income_candidates(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RecurringCandidateDto>, IpcError> {
    income_candidates_impl(state.inner())
}

/// Candidate recurring bills detected from realized outflows: merchants that recur at a
/// consistent cadence + amount, each with an inferred amount/frequency/next-date + confidence.
/// Suggestions for the "Suggested recurring" surface; the user confirms before promotion.
pub fn recurring_candidates_impl(state: &AppState) -> Result<Vec<RecurringCandidateDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .recurring_candidates()?
            .into_iter()
            .map(RecurringCandidateDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn recurring_candidates(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RecurringCandidateDto>, IpcError> {
    recurring_candidates_impl(state.inner())
}

// ---- recurring_bill_history (personal-cfo-4d8.25.8) ------------------------

/// One recurring bill's projected occurrences with their realized links, refreshing the
/// instance seam first (ADR 0047 §1) — the retro-attach surface shown right after a bill
/// is created/approved.
pub fn recurring_bill_history_impl(
    state: &AppState,
    event_id: String,
) -> Result<Vec<RecurringBillOccurrenceDto>, IpcError> {
    let id = RecurringEventId::from_uuid(parse_uuid(&event_id)?);
    with_kernel(state, |kernel| {
        Ok(kernel
            .recurring_bill_history(id)?
            .into_iter()
            .map(RecurringBillOccurrenceDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn recurring_bill_history(
    state: tauri::State<'_, AppState>,
    event_id: String,
) -> Result<Vec<RecurringBillOccurrenceDto>, IpcError> {
    recurring_bill_history_impl(state.inner(), event_id)
}

// ---- check_for_update (personal-cfo-1ik.3) ---------------------------------

/// Check whether a newer build is available in the app's own source checkout — a from-source,
/// dev-machine updater (ADR 0003/0010 addendum). Runs `git fetch` + compares the built commit
/// to the current branch's upstream; degrades to `checked = false` with a reason when it can't
/// run (no checkout / no upstream / git missing). It never fails the IPC layer — the outcome
/// is entirely in the returned status. Needs no vault, so it takes no `AppState`.
#[tauri::command]
#[specta::specta]
pub async fn check_for_update() -> UpdateStatusDto {
    // `async` + `spawn_blocking` keeps the blocking `git` calls off the main thread, so a slow
    // or stalled fetch never freezes the UI (the fetch itself is also time-bounded).
    tauri::async_runtime::spawn_blocking(|| UpdateStatusDto::from(crate::update::check()))
        .await
        .unwrap_or_else(|_| UpdateStatusDto {
            current_version: env!("CARGO_PKG_VERSION").to_owned(),
            current_commit: "unknown".to_owned(),
            build_channel: env!("PCFO_BUILD_CHANNEL").to_owned(),
            build_time: env!("PCFO_BUILD_TIME").to_owned(),
            build_dirty: matches!(env!("PCFO_GIT_DIRTY").as_bytes(), b"true"),
            latest_commit: None,
            commits_behind: None,
            latest_date: None,
            up_to_date: false,
            checked: false,
            error: Some("The update check failed unexpectedly.".to_owned()),
        })
}

// ---- build_info (personal-cfo-4d8.27.3.2) ---------------------------------

/// The running binary's build identity: version, commit, channel, build time, dirty flag.
/// Pure compile-time constants — no git, no network, no vault — so the UI can show which
/// build you are running immediately, including on the lock screen and while offline.
#[tauri::command]
#[specta::specta]
pub fn build_info() -> BuildInfoDto {
    BuildInfoDto {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        commit: env!("PCFO_GIT_COMMIT").to_owned(),
        channel: env!("PCFO_BUILD_CHANNEL").to_owned(),
        built_at: env!("PCFO_BUILD_TIME").to_owned(),
        dirty: matches!(env!("PCFO_GIT_DIRTY").as_bytes(), b"true"),
    }
}

// ---- record_release_update_failure (personal-cfo-md8h2.1) ------------------

/// Record the updater plugin's serialized failure text through the app's redacting tracing
/// boundary. The release updater runs inside the plugin, so this narrow command is the only
/// way its JavaScript-side rejection can enter DohFlow's structured logs. It deliberately takes
/// no vault state and no user-entered context; the frontend passes only the plugin error text
/// and this fixed classification.
pub fn record_release_update_failure_impl(
    error_text: &str,
    failure_kind: ReleaseUpdateFailureKind,
) {
    let started_at = std::time::Instant::now();
    let command_id = Uuid::now_v7();
    let correlation_id = Uuid::now_v7();
    let span = tracing::info_span!(
        "tauri_command",
        command_id = %command_id,
        correlation_id = %correlation_id,
        causation_id = "none",
        actor_type = "user",
        actor_id = "local-user",
        command = "record_release_update_failure",
    );
    let _entered = span.enter();

    // Redact before the event reaches the subscriber as well as at the subscriber itself. This
    // keeps the raw updater diagnostic useful while preserving the no-financial-data log rule if
    // a future test or alternate subscriber observes the event directly.
    let safe_error_text = observability::redact(error_text);
    let duration_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::warn!(
        failure_kind = failure_kind.as_str(),
        error_text = %safe_error_text,
        duration_ms,
        outcome = failure_kind.as_str(),
        "release updater failure recorded",
    );
}

/// Bridge a release-updater failure from the WebView into the redacting tracing subscriber. The
/// command itself cannot fail: losing diagnostic logging must never hide the updater error that
/// the user needs to see.
#[tauri::command]
#[specta::specta]
pub fn record_release_update_failure(error_text: String, failure_kind: ReleaseUpdateFailureKind) {
    record_release_update_failure_impl(&error_text, failure_kind);
}

// ---- apply_update / relaunch_app (personal-cfo-1ik.4) ----------------------

/// Apply an update: rebuild + reinstall the app from the source checkout (via the shared
/// `scripts/update-app.sh`). Minutes-long; `async` + `spawn_blocking` keeps it off the main
/// thread so the UI stays responsive with a progress spinner. Returns whether it succeeded and
/// the tail of its output (for a failure message). Never fails the IPC layer.
#[tauri::command]
#[specta::specta]
pub async fn apply_update() -> ApplyUpdateResultDto {
    tauri::async_runtime::spawn_blocking(|| ApplyUpdateResultDto::from(crate::update::apply()))
        .await
        .unwrap_or_else(|_| ApplyUpdateResultDto {
            ok: false,
            output_tail: "The update failed unexpectedly.".to_owned(),
        })
}

/// Relaunch into the freshly-installed build: spawn a detached helper that reopens the app once
/// this process exits, then exit. Called after `apply_update` succeeds.
#[tauri::command]
#[specta::specta]
pub fn relaunch_app(app: tauri::AppHandle) {
    crate::update::relaunch();
    app.exit(0);
}

// ---- future_cash_by_account (personal-cfo-l8oh) ----------------------------

/// The per-account and per-group Future Cash projection (ADR 0026 §12): one series
/// per liquid account (+ Unallocated) and the cash-tier rollups, reconciling to the
/// aggregate. Backs the multi-series chart + spreadsheet table. `scenario_id` =
/// blank/`None` is the base; a scenario id layers its overlay.
pub fn future_cash_by_account_impl(
    state: &AppState,
    horizon_days: u32,
    scenario_ids: Vec<String>,
) -> Result<MultiSeriesForecastDto, IpcError> {
    let scenarios = parse_scenario_selection(&scenario_ids)?;
    with_kernel(state, |kernel| {
        Ok(MultiSeriesForecastDto::from(
            kernel.future_cash_by_account(horizon_days, &scenarios)?,
        ))
    })
}

#[tauri::command]
#[specta::specta]
pub fn future_cash_by_account(
    state: tauri::State<'_, AppState>,
    horizon_days: u32,
    // The ORDERED scenario selection; empty is the base forecast (ADR 0059 §1).
    scenario_ids: Vec<String>,
) -> Result<MultiSeriesForecastDto, IpcError> {
    future_cash_by_account_impl(state.inner(), horizon_days, scenario_ids)
}

// ---- cash_flow_history (personal-cfo-4d8.27.5.2) ---------------------------

/// The realized cash-flow history over the trailing `lookback_days`: each liquid account's
/// actual daily closing balance, folded backward from today over the ledger postings and
/// clamped to its earliest real data. Backs the Account Detail chart's realized line.
pub fn cash_flow_history_impl(
    state: &AppState,
    lookback_days: u32,
) -> Result<CashFlowHistoryDto, IpcError> {
    with_kernel(state, |kernel| {
        Ok(CashFlowHistoryDto::from(
            kernel.cash_flow_history(lookback_days)?,
        ))
    })
}

#[tauri::command]
#[specta::specta]
pub fn cash_flow_history(
    state: tauri::State<'_, AppState>,
    lookback_days: u32,
) -> Result<CashFlowHistoryDto, IpcError> {
    cash_flow_history_impl(state.inner(), lookback_days)
}

/// How many recent transactions the list surfaces. A personal vault's recent
/// window fits comfortably; deeper history/paging is a follow-up.
const TRANSACTION_LIST_LIMIT: u32 = 200;

/// The most recent transactions, newest first. Backs the transactions list UI.
pub fn transaction_list_impl(state: &AppState) -> Result<Vec<TransactionRowDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .transactions(TRANSACTION_LIST_LIMIT)?
            .into_iter()
            .map(TransactionRowDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn transaction_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<TransactionRowDto>, IpcError> {
    transaction_list_impl(state.inner())
}

/// One filtered, ordered page of transactions plus the total match count
/// (personal-cfo-3fdd.1): server-side search / filter / sort / paging, so the UI
/// reaches all history instead of the recent 200-row window.
pub fn transaction_page_impl(
    state: &AppState,
    input: TransactionPageInput,
) -> Result<TransactionPageDto, IpcError> {
    with_kernel(state, |kernel| {
        let query = input.into_query()?;
        Ok(kernel.transaction_page(&query)?.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn transaction_page(
    state: tauri::State<'_, AppState>,
    input: TransactionPageInput,
) -> Result<TransactionPageDto, IpcError> {
    transaction_page_impl(state.inner(), input)
}

/// Set or clear a transaction's category (ADR 0030, personal-cfo-bac). A manual
/// assignment; `category_id` null clears it (uncategorize).
pub fn recategorize_transaction_impl(
    state: &AppState,
    transaction_id: String,
    category_id: Option<String>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let transaction_id = parse_transaction_id(&transaction_id)?;
        let category_id = match category_id.as_deref() {
            Some(id) => Some(parse_category_id(id)?),
            None => None,
        };
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            RecategorizeTransaction::new(transaction_id, category_id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn recategorize_transaction(
    state: tauri::State<'_, AppState>,
    transaction_id: String,
    category_id: Option<String>,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    recategorize_transaction_impl(state.inner(), transaction_id, category_id, idempotency_key)
}

// ---- income sources (personal-cfo-le79) ------------------------------------

/// Create a recurring net-pay income source.
pub fn create_income_source_impl(
    state: &AppState,
    input: CreateIncomeSourceInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let command = CreateIncomeSource::new(
            input.name.clone(),
            input.net_amount()?,
            input.frequency()?,
            input.anchor()?,
            input.deposit_account_id()?,
        );
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn create_income_source(
    state: tauri::State<'_, AppState>,
    input: CreateIncomeSourceInput,
) -> Result<MutationResult, IpcError> {
    create_income_source_impl(state.inner(), input)
}

/// Every income source with its next pay date. Backs the income list UI.
pub fn income_source_list_impl(state: &AppState) -> Result<Vec<IncomeSourceDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .income_source_views()?
            .into_iter()
            .map(IncomeSourceDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn income_source_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<IncomeSourceDto>, IpcError> {
    income_source_list_impl(state.inner())
}

// ---- income source edit / delete / archive (personal-cfo-tch0) --------------

/// Edit an existing net-pay income source.
pub fn update_income_source_impl(
    state: &AppState,
    input: UpdateIncomeSourceInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let command = UpdateIncomeSource::new(
            input.income_source_id()?,
            input.name.clone(),
            input.net_amount()?,
            input.frequency()?,
            input.anchor()?,
            input.deposit_account_id()?,
        );
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn update_income_source(
    state: tauri::State<'_, AppState>,
    input: UpdateIncomeSourceInput,
) -> Result<MutationResult, IpcError> {
    update_income_source_impl(state.inner(), input)
}

/// Delete a net-pay income source.
pub fn delete_income_source_impl(
    state: &AppState,
    income_source_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_income_source_id(&income_source_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            DeleteIncomeSource::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn delete_income_source(
    state: tauri::State<'_, AppState>,
    income_source_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    delete_income_source_impl(state.inner(), income_source_id, idempotency_key)
}

/// Archive a net-pay income source (soft-hide: leaves the active list + forecast,
/// retained with its dates, restorable).
pub fn archive_income_source_impl(
    state: &AppState,
    income_source_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_income_source_id(&income_source_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            ArchiveIncomeSource::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn archive_income_source(
    state: tauri::State<'_, AppState>,
    income_source_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    archive_income_source_impl(state.inner(), income_source_id, idempotency_key)
}

/// Restore a previously archived income source.
pub fn restore_income_source_impl(
    state: &AppState,
    income_source_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_income_source_id(&income_source_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            RestoreIncomeSource::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn restore_income_source(
    state: tauri::State<'_, AppState>,
    income_source_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    restore_income_source_impl(state.inner(), income_source_id, idempotency_key)
}

// ---- recurring bills (personal-cfo-esmy) -----------------------------------

/// Create a manual recurring bill (writes a recurring event + bill contract and
/// refreshes the commitments projection).
pub fn create_recurring_bill_impl(
    state: &AppState,
    input: CreateRecurringBillInput,
) -> Result<CreateRecurringBillResult, IpcError> {
    with_kernel(state, |kernel| {
        // Mint the id up front so autopay can be set on the new bill in the same call (ADR 0041).
        let event_id = RecurringEventId::new();
        let category_id = input
            .category_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(parse_category_id)
            .transpose()?;
        let tag_ids = input
            .tag_ids
            .iter()
            .map(|t| parse_tag_id(t))
            .collect::<Result<Vec<_>, _>>()?;
        let command = CreateRecurringBill::new(
            event_id,
            input.name.clone(),
            input.amount()?,
            input.bill_type()?,
            input.frequency()?,
            input.anchor()?,
            input.autopay_account_id()?,
            input.description(),
        )
        .with_source_merchant_key(input.source_merchant_key.clone())
        .with_category_id(category_id)
        .with_tag_ids(tag_ids);
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        // Autopay intent is a follow-up command on the new bill (blank key → a fresh one).
        if input.autopay.unwrap_or(false) {
            kernel.dispatch(CommandEnvelope::new(
                user_meta(""),
                SetBillAutopay::new(event_id, true),
            ))?;
        }
        let result: MutationResult = outcome.into();
        // On an idempotent REPLAY the freshly-minted id was never written — the original
        // call's bill exists under its own id — so the returned id resolves to an empty
        // history (harmless: the retro-attach panel simply doesn't show).
        Ok(CreateRecurringBillResult {
            op_seq: result.op_seq,
            replayed: result.replayed,
            event_id: event_id.as_uuid().to_string(),
        })
    })
}

#[tauri::command]
#[specta::specta]
pub fn create_recurring_bill(
    state: tauri::State<'_, AppState>,
    input: CreateRecurringBillInput,
) -> Result<CreateRecurringBillResult, IpcError> {
    create_recurring_bill_impl(state.inner(), input)
}

// ---- confirm/unconfirm obligation (personal-cfo-5ie.9) ---------------------

/// Mark a recurring bill occurrence paid early: posts a real liquid outflow and stops the
/// forecast from projecting that occurrence (the pay-&-confirm loop).
pub fn confirm_obligation_early_impl(
    state: &AppState,
    input: ConfirmObligationEarlyInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let command = ConfirmObligationEarly::new(
            input.recurring_event_id()?,
            input.scheduled_date()?,
            input.actual_amount.to_money()?,
            input.actual_date()?,
            input.paying_account_id()?,
        );
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn confirm_obligation_early(
    state: tauri::State<'_, AppState>,
    input: ConfirmObligationEarlyInput,
) -> Result<MutationResult, IpcError> {
    confirm_obligation_early_impl(state.inner(), input)
}

/// Reverse an early confirm: void the posted transaction and let the occurrence project again.
pub fn unconfirm_obligation_impl(
    state: &AppState,
    input: UnconfirmObligationInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let command =
            UnconfirmObligation::new(input.recurring_event_id()?, input.scheduled_date()?);
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn unconfirm_obligation(
    state: tauri::State<'_, AppState>,
    input: UnconfirmObligationInput,
) -> Result<MutationResult, IpcError> {
    unconfirm_obligation_impl(state.inner(), input)
}

/// Mark a recurring bill autopay or manual (ADR 0041, personal-cfo-mc7f).
pub fn set_bill_autopay_impl(
    state: &AppState,
    input: SetBillAutopayInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let command = SetBillAutopay::new(input.event_id()?, input.autopay);
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_bill_autopay(
    state: tauri::State<'_, AppState>,
    input: SetBillAutopayInput,
) -> Result<MutationResult, IpcError> {
    set_bill_autopay_impl(state.inner(), input)
}

/// Every manual recurring bill with its next due date. Backs the bills list UI.
pub fn recurring_bill_list_impl(state: &AppState) -> Result<Vec<RecurringBillDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .recurring_bill_views()?
            .into_iter()
            .map(RecurringBillDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn recurring_bill_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RecurringBillDto>, IpcError> {
    recurring_bill_list_impl(state.inner())
}

/// Create a recurring transfer (ADR 0026 §14, personal-cfo-npoe). The id is minted here.
pub fn create_recurring_transfer_impl(
    state: &AppState,
    input: CreateRecurringTransferInput,
) -> Result<CreateRecurringTransferResult, IpcError> {
    with_kernel(state, |kernel| {
        let source = parse_account_id(&input.source_account_id)?;
        let dest = parse_account_id(&input.dest_account_id)?;
        let amount = input.amount.to_money()?;
        let id = RecurringTransferId::new();
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            CreateRecurringTransfer::new(
                id,
                source,
                dest,
                amount,
                input.frequency()?,
                input.anchor()?,
            ),
        ))?;
        Ok(CreateRecurringTransferResult {
            recurring_transfer_id: id.to_string(),
            mutation: outcome.into(),
        })
    })
}

#[tauri::command]
#[specta::specta]
pub fn create_recurring_transfer(
    state: tauri::State<'_, AppState>,
    input: CreateRecurringTransferInput,
) -> Result<CreateRecurringTransferResult, IpcError> {
    create_recurring_transfer_impl(state.inner(), input)
}

/// Every recurring transfer with its next occurrence (personal-cfo-npoe).
pub fn recurring_transfer_list_impl(
    state: &AppState,
) -> Result<Vec<RecurringTransferDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .recurring_transfer_views()?
            .into_iter()
            .map(RecurringTransferDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn recurring_transfer_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RecurringTransferDto>, IpcError> {
    recurring_transfer_list_impl(state.inner())
}

/// Delete a recurring transfer (personal-cfo-npoe). Future projection stops; any
/// already-posted one-off transfers are untouched.
pub fn delete_recurring_transfer_impl(
    state: &AppState,
    recurring_transfer_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_recurring_transfer_id(&recurring_transfer_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            DeleteRecurringTransfer::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn delete_recurring_transfer(
    state: tauri::State<'_, AppState>,
    recurring_transfer_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    delete_recurring_transfer_impl(state.inner(), recurring_transfer_id, idempotency_key)
}

/// The full category taxonomy (plan §9.6, ADR 0030, personal-cfo-bac) — the seeded
/// defaults plus any user categories, for the management UI + the picker.
pub fn category_list_impl(state: &AppState) -> Result<Vec<CategoryDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .category_views()?
            .into_iter()
            .map(CategoryDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn category_list(state: tauri::State<'_, AppState>) -> Result<Vec<CategoryDto>, IpcError> {
    category_list_impl(state.inner())
}

/// Create a user category (ADR 0030, personal-cfo-bac). The id is minted here.
pub fn create_category_impl(
    state: &AppState,
    input: CreateCategoryInput,
) -> Result<CreateCategoryResult, IpcError> {
    with_kernel(state, |kernel| {
        let parent_id = match input.parent_id.as_deref() {
            Some(p) => Some(parse_category_id(p)?),
            None => None,
        };
        let id = CategoryId::new();
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            CreateCategory::new(
                id,
                parent_id,
                input.name,
                input.category_type,
                input.color,
                input.icon,
            ),
        ))?;
        Ok(CreateCategoryResult {
            category_id: id.to_string(),
            mutation: outcome.into(),
        })
    })
}

#[tauri::command]
#[specta::specta]
pub fn create_category(
    state: tauri::State<'_, AppState>,
    input: CreateCategoryInput,
) -> Result<CreateCategoryResult, IpcError> {
    create_category_impl(state.inner(), input)
}

/// Edit a category (ADR 0030, personal-cfo-bac). A user category updates name + color +
/// icon; a system ("Default") category updates its appearance (color + icon) only — its
/// name is preserved and a submitted name is ignored (ADR 0030 amendment, personal-cfo-kogu).
pub fn update_category_impl(
    state: &AppState,
    input: UpdateCategoryInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_category_id(&input.id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            UpdateCategory::new(id, input.name, input.color, input.icon),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn update_category(
    state: tauri::State<'_, AppState>,
    input: UpdateCategoryInput,
) -> Result<MutationResult, IpcError> {
    update_category_impl(state.inner(), input)
}

/// Re-parent a user category, or make it a top-level group (`new_parent_id` null)
/// (ADR 0030, personal-cfo-bac). Cycles and system categories are rejected.
pub fn move_category_impl(
    state: &AppState,
    input: MoveCategoryInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_category_id(&input.id)?;
        let new_parent_id = match input.new_parent_id.as_deref() {
            Some(p) => Some(parse_category_id(p)?),
            None => None,
        };
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            MoveCategory::new(id, new_parent_id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn move_category(
    state: tauri::State<'_, AppState>,
    input: MoveCategoryInput,
) -> Result<MutationResult, IpcError> {
    move_category_impl(state.inner(), input)
}

/// Archive (hide) a category — system or user (ADR 0030, personal-cfo-bac).
pub fn archive_category_impl(
    state: &AppState,
    category_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_category_id(&category_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            ArchiveCategory::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn archive_category(
    state: tauri::State<'_, AppState>,
    category_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    archive_category_impl(state.inner(), category_id, idempotency_key)
}

/// Un-archive a previously hidden category (ADR 0030, personal-cfo-bac).
pub fn reinstate_category_impl(
    state: &AppState,
    category_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_category_id(&category_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            ReinstateCategory::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn reinstate_category(
    state: tauri::State<'_, AppState>,
    category_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    reinstate_category_impl(state.inner(), category_id, idempotency_key)
}

/// Every active Money Inbox item — the triage surface for import/commit
/// exceptions the pipeline could not auto-resolve (ADR 0014 §7, personal-cfo-dsq).
pub fn money_inbox_list_impl(state: &AppState) -> Result<Vec<MoneyInboxItemDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .money_inbox_list()?
            .into_iter()
            .map(MoneyInboxItemDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn money_inbox_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<MoneyInboxItemDto>, IpcError> {
    money_inbox_list_impl(state.inner())
}

/// Bulk-accept the low-confidence-category review queue (ADR 0030 addendum,
/// personal-cfo-j5ij): mark every queued transaction reviewed (keeping its rule category),
/// clearing them from the Money Inbox. Returns the number accepted.
pub fn accept_low_confidence_categories_impl(
    state: &AppState,
    idempotency_key: String,
) -> Result<u32, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel.accept_low_confidence_categories(&user_meta(&idempotency_key))?)
    })
}

#[tauri::command]
#[specta::specta]
pub fn accept_low_confidence_categories(
    state: tauri::State<'_, AppState>,
    idempotency_key: String,
) -> Result<u32, IpcError> {
    accept_low_confidence_categories_impl(state.inner(), idempotency_key)
}

// ---- mark_inbox_reviewed_bulk (personal-cfo-4d8.25.16) ----------------------

/// Bulk mark an explicit transaction set reviewed in ONE round-trip: the kernel
/// dispatches one audited `MarkReviewed` per id, all sharing the seed's correlation id
/// (the `accept_low_confidence_categories` shape). Returns the number marked.
pub fn mark_inbox_reviewed_bulk_impl(
    state: &AppState,
    idempotency_key: String,
    transaction_ids: Vec<String>,
) -> Result<u32, IpcError> {
    let ids = transaction_ids
        .iter()
        .map(|id| parse_transaction_id(id))
        .collect::<Result<Vec<_>, _>>()?;
    with_kernel(state, |kernel| {
        Ok(kernel.mark_transactions_reviewed_bulk(&user_meta(&idempotency_key), &ids)?)
    })
}

#[tauri::command]
#[specta::specta]
pub fn mark_inbox_reviewed_bulk(
    state: tauri::State<'_, AppState>,
    idempotency_key: String,
    transaction_ids: Vec<String>,
) -> Result<u32, IpcError> {
    mark_inbox_reviewed_bulk_impl(state.inner(), idempotency_key, transaction_ids)
}

// ---- transaction_rows_by_ids (personal-cfo-4d8.25.15) -----------------------

/// The row DTOs for an explicit id set — resolves Money Inbox items outside the
/// recent-window list for selection, bulk actions, and Card Review. Unknown or voided
/// ids are absent from the result.
pub fn transaction_rows_by_ids_impl(
    state: &AppState,
    transaction_ids: Vec<String>,
) -> Result<Vec<TransactionRowDto>, IpcError> {
    let ids = transaction_ids
        .iter()
        .map(|id| parse_transaction_id(id))
        .collect::<Result<Vec<_>, _>>()?;
    with_kernel(state, |kernel| {
        Ok(kernel
            .transactions_by_ids(&ids)?
            .into_iter()
            .map(TransactionRowDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn transaction_rows_by_ids(
    state: tauri::State<'_, AppState>,
    transaction_ids: Vec<String>,
) -> Result<Vec<TransactionRowDto>, IpcError> {
    transaction_rows_by_ids_impl(state.inner(), transaction_ids)
}

/// Money Inbox "import anyway": force-commit a flagged staged transaction to the
/// ledger despite the suspected duplicate (ADR 0014 §7, personal-cfo-asqy). The
/// committed row leaves `flagged`, so the inbox item clears on the next rebuild.
pub fn import_staged_anyway_impl(
    state: &AppState,
    staged_transaction_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_staged_transaction_id(&staged_transaction_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            CommitStaged::import_anyway(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn import_staged_anyway(
    state: tauri::State<'_, AppState>,
    staged_transaction_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    import_staged_anyway_impl(state.inner(), staged_transaction_id, idempotency_key)
}

/// Money Inbox "skip": mark a flagged staged transaction skipped without a ledger
/// write (ADR 0014 §7, personal-cfo-asqy). The skipped row leaves `flagged`, so
/// the inbox item clears on the next rebuild.
pub fn skip_staged_transaction_impl(
    state: &AppState,
    staged_transaction_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_staged_transaction_id(&staged_transaction_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            SkipStaged::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn skip_staged_transaction(
    state: tauri::State<'_, AppState>,
    staged_transaction_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    skip_staged_transaction_impl(state.inner(), staged_transaction_id, idempotency_key)
}

/// Snooze a Money Inbox item until `until_date` (`YYYY-MM-DD`) — hide it from the
/// default list until then (ADR 0014 §7, personal-cfo-ci71).
pub fn snooze_inbox_item_impl(
    state: &AppState,
    item_id: String,
    until_date: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = Uuid::parse_str(item_id.trim())
            .map_err(|_| IpcError::Validation(format!("not a valid inbox item id: {item_id:?}")))?;
        let until = NaiveDate::parse_from_str(until_date.trim(), "%Y-%m-%d")
            .map_err(|_| IpcError::Validation(format!("not a valid date: {until_date:?}")))?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            SnoozeInboxItem::new(id, until),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn snooze_inbox_item(
    state: tauri::State<'_, AppState>,
    item_id: String,
    until_date: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    snooze_inbox_item_impl(state.inner(), item_id, until_date, idempotency_key)
}

/// Dismiss a Money Inbox item with a typed `reason` (ADR 0014 §7, personal-cfo-ci71).
pub fn dismiss_inbox_item_impl(
    state: &AppState,
    item_id: String,
    reason: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = Uuid::parse_str(item_id.trim())
            .map_err(|_| IpcError::Validation(format!("not a valid inbox item id: {item_id:?}")))?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            DismissInboxItem::new(id, reason),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn dismiss_inbox_item(
    state: tauri::State<'_, AppState>,
    item_id: String,
    reason: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    dismiss_inbox_item_impl(state.inner(), item_id, reason, idempotency_key)
}

/// Dismiss a recurring-bill SUGGESTION (ADR 0046, personal-cfo-4d8.24.6): record a
/// `(merchant_key, currency)` suppression at the dismissed amount + cadence so detection
/// stops offering it until the pattern materially changes (amount outside the band, or a
/// different cadence). Idempotent per key; latest dismiss wins.
pub fn dismiss_recurring_suggestion_impl(
    state: &AppState,
    input: DismissRecurringSuggestionInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            DismissRecurringSuggestion::new(
                input.merchant_key,
                input.currency,
                input.amount_minor,
                input.frequency,
                input.reason,
            ),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn dismiss_recurring_suggestion(
    state: tauri::State<'_, AppState>,
    input: DismissRecurringSuggestionInput,
) -> Result<MutationResult, IpcError> {
    dismiss_recurring_suggestion_impl(state.inner(), input)
}

/// Edit an existing manual recurring bill (rewrites its recurring event + bill
/// contract and refreshes the commitments projection).
pub fn update_recurring_bill_impl(
    state: &AppState,
    input: UpdateRecurringBillInput,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let command = UpdateRecurringBill::new(
            input.bill_id()?,
            input.name.clone(),
            input.amount()?,
            input.bill_type()?,
            input.frequency()?,
            input.anchor()?,
            input.autopay_account_id()?,
            input.description(),
        );
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&input.idempotency_key),
            command,
        ))?;
        // Apply the autopay intent when the caller provides it (ADR 0041). The bill form always
        // sends the current value, so an edit re-affirms it (idempotent); a caller that omits it
        // leaves autopay untouched.
        if let Some(autopay) = input.autopay {
            kernel.dispatch(CommandEnvelope::new(
                user_meta(""),
                SetBillAutopay::new(input.bill_id()?, autopay),
            ))?;
        }
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn update_recurring_bill(
    state: tauri::State<'_, AppState>,
    input: UpdateRecurringBillInput,
) -> Result<MutationResult, IpcError> {
    update_recurring_bill_impl(state.inner(), input)
}

/// Delete a manual recurring bill (removes its recurring event + bill contract and
/// refreshes the commitments projection).
pub fn delete_recurring_bill_impl(
    state: &AppState,
    bill_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_recurring_event_id(&bill_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            DeleteRecurringBill::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn delete_recurring_bill(
    state: tauri::State<'_, AppState>,
    bill_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    delete_recurring_bill_impl(state.inner(), bill_id, idempotency_key)
}

/// Archive a recurring bill (personal-cfo-4d8.2): soft-hide it — it leaves the
/// active list + forecast but is retained (with its dates) and restorable.
pub fn archive_recurring_bill_impl(
    state: &AppState,
    bill_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_recurring_event_id(&bill_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            ArchiveRecurringBill::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn archive_recurring_bill(
    state: tauri::State<'_, AppState>,
    bill_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    archive_recurring_bill_impl(state.inner(), bill_id, idempotency_key)
}

/// Restore a previously archived recurring bill (personal-cfo-4d8.2).
pub fn restore_recurring_bill_impl(
    state: &AppState,
    bill_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_recurring_event_id(&bill_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            RestoreRecurringBill::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn restore_recurring_bill(
    state: tauri::State<'_, AppState>,
    bill_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    restore_recurring_bill_impl(state.inner(), bill_id, idempotency_key)
}

// ---- settings: base/reporting currency (personal-cfo-p5g / -4d8.1) ----------

/// The settings key holding the vault's base/reporting currency code.
const REPORTING_CURRENCY_KEY: &str = "reporting_currency";

/// The vault's base/reporting currency code, defaulting to `"USD"` when it has
/// never been set. New account/income/bill forms default to this instead of
/// forcing a per-entry currency pick (personal-cfo-4d8.1).
pub fn base_currency_impl(state: &AppState) -> Result<String, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .get_setting(REPORTING_CURRENCY_KEY)?
            .unwrap_or_else(|| "USD".to_owned()))
    })
}

#[tauri::command]
#[specta::specta]
pub fn base_currency(state: tauri::State<'_, AppState>) -> Result<String, IpcError> {
    base_currency_impl(state.inner())
}

/// Set the vault's base/reporting currency. The code is validated against the
/// supported set and stored canonically (e.g. `"usd"` → `"USD"`).
pub fn set_base_currency_impl(state: &AppState, code: String) -> Result<(), IpcError> {
    let currency = parse_currency(&code)?;
    with_kernel(state, |kernel| {
        kernel.set_setting(REPORTING_CURRENCY_KEY, currency.code())?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_base_currency(state: tauri::State<'_, AppState>, code: String) -> Result<(), IpcError> {
    set_base_currency_impl(state.inner(), code)
}

// ---- manual future entries (personal-cfo-q6gh) -----------------------------
//
// Forecast inputs (5u2 assumption events), not ledger writes, so — like the
// settings commands — they use the direct-write `with_kernel` shape, not the
// `WriteCommand` bus.

/// Parse a UUID string id (an assumption-event id) into a [`Uuid`].
fn parse_uuid(raw: &str) -> Result<Uuid, IpcError> {
    Uuid::parse_str(raw.trim()).map_err(|_| IpcError::Validation(format!("not a valid id: {raw}")))
}

/// Parse a scenario lifecycle token (`draft` / `active` / `archived`).
fn parse_scenario_status(raw: &str) -> Result<ScenarioStatus, IpcError> {
    ScenarioStatus::from_token(raw.trim())
        .ok_or_else(|| IpcError::Validation(format!("not a valid scenario status: {raw}")))
}

/// Create a manual future entry — a one-time signed cash event on a date.
pub fn create_manual_future_entry_impl(
    state: &AppState,
    input: CreateManualFutureEntryInput,
) -> Result<ManualFutureEntryDto, IpcError> {
    let amount = input.amount.to_money()?;
    let occurs_on = input.occurs_on()?;
    let account_id = parse_optional_account(&input.account_id)?;
    let id = Uuid::now_v7();
    with_kernel(state, |kernel| {
        kernel.record_manual_entry(id, amount, occurs_on, &input.label, account_id)?;
        Ok(ManualFutureEntryDto {
            id: id.to_string(),
            amount: MoneyDto::from(amount),
            date: occurs_on.to_string(),
            label: input.label.clone(),
            account_id: account_id.map(|a| a.to_string()),
            // A just-created/edited entry has not been through the matcher.
            matched_transaction_id: None,
        })
    })
}

/// Parse an optional account-id string (blank ⇒ `None`, invalid ⇒ a validation error).
fn parse_optional_account(raw: &Option<String>) -> Result<Option<Uuid>, IpcError> {
    raw.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_uuid)
        .transpose()
}

#[tauri::command]
#[specta::specta]
pub fn create_manual_future_entry(
    state: tauri::State<'_, AppState>,
    input: CreateManualFutureEntryInput,
) -> Result<ManualFutureEntryDto, IpcError> {
    create_manual_future_entry_impl(state.inner(), input)
}

/// List the active manual future entries (oldest first).
pub fn manual_future_entry_list_impl(
    state: &AppState,
) -> Result<Vec<ManualFutureEntryDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .manual_entries()?
            .into_iter()
            .map(ManualFutureEntryDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn manual_future_entry_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ManualFutureEntryDto>, IpcError> {
    manual_future_entry_list_impl(state.inner())
}

/// Edit a manual future entry: record a replacement that supersedes the prior one
/// (no silent mutation; history retained).
pub fn update_manual_future_entry_impl(
    state: &AppState,
    input: UpdateManualFutureEntryInput,
) -> Result<ManualFutureEntryDto, IpcError> {
    let prior = parse_uuid(&input.id)?;
    let amount = input.amount.to_money()?;
    let occurs_on = input.occurs_on()?;
    let account_id = parse_optional_account(&input.account_id)?;
    let id = Uuid::now_v7();
    with_kernel(state, |kernel| {
        kernel.record_manual_entry(id, amount, occurs_on, &input.label, account_id)?;
        kernel.supersede_assumption_event(prior, id)?;
        Ok(ManualFutureEntryDto {
            id: id.to_string(),
            amount: MoneyDto::from(amount),
            date: occurs_on.to_string(),
            label: input.label.clone(),
            account_id: account_id.map(|a| a.to_string()),
            // A just-created/edited entry has not been through the matcher.
            matched_transaction_id: None,
        })
    })
}

#[tauri::command]
#[specta::specta]
pub fn update_manual_future_entry(
    state: tauri::State<'_, AppState>,
    input: UpdateManualFutureEntryInput,
) -> Result<ManualFutureEntryDto, IpcError> {
    update_manual_future_entry_impl(state.inner(), input)
}

/// Delete a manual future entry: clear it (the row is retained, just deactivated).
pub fn delete_manual_future_entry_impl(state: &AppState, id: String) -> Result<(), IpcError> {
    let id = parse_uuid(&id)?;
    with_kernel(state, |kernel| {
        kernel.clear_assumption_event(id)?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn delete_manual_future_entry(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), IpcError> {
    delete_manual_future_entry_impl(state.inner(), id)
}

// ===== Scenarios + forecast assumptions (ADR 0026 §5, personal-cfo-6zep) =====
//
// A scenario is a named overlay (0mg `scenarios` row + scenario-scoped 5u2
// assumption events); a forecast assumption is one such event (addition,
// modification, or removal). Both are forecast inputs, not ledger writes, so they
// use the direct-write `with_kernel` shape (the q6gh manual-entry template).

/// Create a scenario (status `draft`) and return it.
pub fn create_scenario_impl(
    state: &AppState,
    input: CreateScenarioInput,
) -> Result<ScenarioDto, IpcError> {
    let id = Uuid::now_v7();
    with_kernel(state, |kernel| {
        kernel.create_scenario(&NewScenario {
            id,
            name: input.name.clone(),
            description: input.description.clone(),
            base_run_id: None,
        })?;
        let view = kernel
            .scenario(id)?
            .ok_or_else(|| IpcError::Persistence("scenario was not recorded".to_owned()))?;
        Ok(ScenarioDto::from(view))
    })
}

#[tauri::command]
#[specta::specta]
pub fn create_scenario(
    state: tauri::State<'_, AppState>,
    input: CreateScenarioInput,
) -> Result<ScenarioDto, IpcError> {
    create_scenario_impl(state.inner(), input)
}

/// List all scenarios, oldest first.
pub fn scenario_list_impl(state: &AppState) -> Result<Vec<ScenarioDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .list_scenarios()?
            .into_iter()
            .map(ScenarioDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn scenario_list(state: tauri::State<'_, AppState>) -> Result<Vec<ScenarioDto>, IpcError> {
    scenario_list_impl(state.inner())
}

/// Update a scenario's lifecycle status (draft → active → archived).
pub fn update_scenario_impl(
    state: &AppState,
    input: UpdateScenarioInput,
) -> Result<ScenarioDto, IpcError> {
    let id = parse_uuid(&input.id)?;
    let status = parse_scenario_status(&input.status)?;
    with_kernel(state, |kernel| {
        if let Some(name) = &input.name {
            kernel.rename_scenario(id, name)?;
        }
        kernel.set_scenario_status(id, status)?;
        let view = kernel
            .scenario(id)?
            .ok_or_else(|| IpcError::Validation(format!("no scenario with id {}", input.id)))?;
        Ok(ScenarioDto::from(view))
    })
}

#[tauri::command]
#[specta::specta]
pub fn update_scenario(
    state: tauri::State<'_, AppState>,
    input: UpdateScenarioInput,
) -> Result<ScenarioDto, IpcError> {
    update_scenario_impl(state.inner(), input)
}

/// Delete a scenario (soft): archive it and clear its active scenario-scoped
/// events so it neither appears nor affects any forecast.
pub fn delete_scenario_impl(state: &AppState, id: String) -> Result<(), IpcError> {
    let id = parse_uuid(&id)?;
    with_kernel(state, |kernel| {
        kernel.delete_scenario(id)?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn delete_scenario(state: tauri::State<'_, AppState>, id: String) -> Result<(), IpcError> {
    delete_scenario_impl(state.inner(), id)
}

/// Spend rolled up by category over a date range (ADR 0052, personal-cfo-4d8.27.8.2).
pub fn spend_by_category_impl(
    state: &AppState,
    input: SpendByCategoryInput,
) -> Result<SpendBreakdownDto, IpcError> {
    let from = super::dto::parse_iso_date(&input.from)?;
    let to = super::dto::parse_iso_date(&input.to)?;
    if to < from {
        return Err(IpcError::Validation(
            "the end date must be on or after the start date".to_owned(),
        ));
    }
    let parent = parse_opt_uuid(input.parent_id.as_deref())?;
    // The list's facets, so the chart counts exactly the rows the list would show
    // (ADR 0052 §2). The category facet is absent by design — it is `parent` above.
    let filters = SpendFilters {
        query: input.query,
        account_ids: input
            .account_ids
            .iter()
            .map(|raw| raw.trim())
            .filter(|raw| !raw.is_empty())
            .map(parse_uuid)
            .collect::<Result<Vec<_>, _>>()?,
        tag_id: parse_opt_uuid(input.tag_id.as_deref())?,
        unreviewed_only: input.unreviewed_only,
    };
    with_kernel(state, |kernel| {
        let currency = kernel
            .get_setting(REPORTING_CURRENCY_KEY)?
            .unwrap_or_else(|| "USD".to_owned());
        Ok(SpendBreakdownDto::from(kernel.spend_by_category(
            from, to, parent, &currency, &filters,
        )?))
    })
}

#[tauri::command]
#[specta::specta]
pub fn spend_by_category(
    state: tauri::State<'_, AppState>,
    input: SpendByCategoryInput,
) -> Result<SpendBreakdownDto, IpcError> {
    spend_by_category_impl(state.inner(), input)
}

/// Archive a scenario, keeping its events (ADR 0051 §1). Reversible via `update_scenario`.
pub fn archive_scenario_impl(state: &AppState, id: String) -> Result<(), IpcError> {
    let id = parse_uuid(&id)?;
    with_kernel(state, |kernel| {
        kernel.archive_scenario(id)?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn archive_scenario(state: tauri::State<'_, AppState>, id: String) -> Result<(), IpcError> {
    archive_scenario_impl(state.inner(), id)
}

/// Promote a scenario's active events into base (ADR 0055 §1, personal-cfo-4d8.27.6.3).
///
/// This is the one scenario operation that changes the household's real forecast. It
/// promotes EVENTS; it never rewrites bills, income sources, or any ledger row.
pub fn apply_scenario_impl(state: &AppState, id: String) -> Result<(), IpcError> {
    let id = parse_uuid(&id)?;
    with_kernel(state, |kernel| {
        kernel.dispatch(CommandEnvelope::new(
            // A FRESH key per invocation, deliberately. A key derived from the scenario
            // id would be stable, and `find_by_idempotency_key` replays a known key —
            // so after apply → revert, a second apply would be memoized as a replay and
            // silently do nothing. Double-apply is already prevented where it belongs:
            // the db-worker rejects a scenario that is already applied (ADR 0055).
            user_meta(""),
            ApplyScenario::new(id),
        ))?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn apply_scenario(state: tauri::State<'_, AppState>, id: String) -> Result<(), IpcError> {
    apply_scenario_impl(state.inner(), id)
}

/// Undo an apply (ADR 0055 §5): clear the promoted base events and restore whatever they
/// superseded. An error on a scenario that is not applied, deliberately.
pub fn revert_scenario_apply_impl(state: &AppState, id: String) -> Result<(), IpcError> {
    let id = parse_uuid(&id)?;
    with_kernel(state, |kernel| {
        kernel.dispatch(CommandEnvelope::new(
            // Fresh per invocation, for the same reason as apply above: revert → apply →
            // revert must work, and the "not applied" guard is what makes a double-revert
            // safe.
            user_meta(""),
            RevertScenarioApply::new(id),
        ))?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn revert_scenario_apply(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), IpcError> {
    revert_scenario_apply_impl(state.inner(), id)
}

/// Clone a scenario into a new draft with a copy of its events (ADR 0051 §2).
pub fn clone_scenario_impl(
    state: &AppState,
    input: CloneScenarioInput,
) -> Result<String, IpcError> {
    let id = parse_uuid(&input.id)?;
    with_kernel(state, |kernel| {
        Ok(kernel.clone_scenario(id, &input.name)?.to_string())
    })
}

#[tauri::command]
#[specta::specta]
pub fn clone_scenario(
    state: tauri::State<'_, AppState>,
    input: CloneScenarioInput,
) -> Result<String, IpcError> {
    clone_scenario_impl(state.inner(), input)
}

/// Set or clear a scenario's expiry date (ADR 0051 §3).
pub fn set_scenario_expiry_impl(
    state: &AppState,
    input: SetScenarioExpiryInput,
) -> Result<(), IpcError> {
    let id = parse_uuid(&input.id)?;
    with_kernel(state, |kernel| {
        kernel.set_scenario_expiry(id, input.expires_on.as_deref())?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_scenario_expiry(
    state: tauri::State<'_, AppState>,
    input: SetScenarioExpiryInput,
) -> Result<(), IpcError> {
    set_scenario_expiry_impl(state.inner(), input)
}

/// Create a forecast assumption event (addition, modification, or removal), base
/// or scenario-scoped, and return the stored event.
pub fn create_forecast_assumption_impl(
    state: &AppState,
    input: CreateForecastAssumptionInput,
) -> Result<AssumptionEventDto, IpcError> {
    let id = Uuid::now_v7();
    let spec = input.to_spec(id)?;
    let scenario = spec.scenario_id;
    with_kernel(state, |kernel| {
        kernel.record_forecast_assumption(&spec)?;
        let view = kernel
            .active_assumption_events(scenario)?
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| IpcError::Persistence("assumption was not recorded".to_owned()))?;
        Ok(AssumptionEventDto::from(view))
    })
}

#[tauri::command]
#[specta::specta]
pub fn create_forecast_assumption(
    state: tauri::State<'_, AppState>,
    input: CreateForecastAssumptionInput,
) -> Result<AssumptionEventDto, IpcError> {
    create_forecast_assumption_impl(state.inner(), input)
}

/// List the active forecast assumption events for a scenario: `None`/blank = the
/// base assumptions; a scenario id = exactly that scenario's events.
pub fn forecast_assumption_list_impl(
    state: &AppState,
    scenario_id: Option<String>,
) -> Result<Vec<AssumptionEventDto>, IpcError> {
    let scenario = parse_opt_uuid(scenario_id.as_deref())?;
    with_kernel(state, |kernel| {
        Ok(kernel
            .active_assumption_events(scenario)?
            .into_iter()
            .map(AssumptionEventDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn forecast_assumption_list(
    state: tauri::State<'_, AppState>,
    scenario_id: Option<String>,
) -> Result<Vec<AssumptionEventDto>, IpcError> {
    forecast_assumption_list_impl(state.inner(), scenario_id)
}

/// Delete a forecast assumption event: clear it (the row is retained, just
/// deactivated).
pub fn delete_forecast_assumption_impl(state: &AppState, id: String) -> Result<(), IpcError> {
    let id = parse_uuid(&id)?;
    with_kernel(state, |kernel| {
        kernel.clear_assumption_event(id)?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn delete_forecast_assumption(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), IpcError> {
    delete_forecast_assumption_impl(state.inner(), id)
}

// ===== Balance assertions (ADR 0027, personal-cfo-ueg6) =====
//
// The additive set-balance: assert an account's balance directly (no transaction),
// stored as a manual balance observation. A forecast/balance input, so it uses the
// direct-write `with_kernel` shape, not the `WriteCommand` bus.

/// Assert an account's balance directly as of a date (no transaction). Returns the
/// new assertion-anchored balance and the still-unexplained adjustment.
pub fn assert_balance_impl(
    state: &AppState,
    input: AssertBalanceInput,
) -> Result<AssertBalanceResult, IpcError> {
    let account = input.account()?;
    let amount = input.amount.to_money()?;
    let as_of = input.as_of()?;
    let id = Uuid::now_v7();
    with_kernel(state, |kernel| {
        kernel.record_balance_assertion(id, account, amount, as_of)?;
        let balance = kernel
            .account_balance(account)?
            .ok_or_else(|| IpcError::Validation("no such account".to_owned()))?;
        let unexplained = kernel.account_unexplained(account)?;
        Ok(AssertBalanceResult {
            balance: MoneyDto::from(balance),
            unexplained: unexplained.map(MoneyDto::from),
        })
    })
}

#[tauri::command]
#[specta::specta]
pub fn assert_balance(
    state: tauri::State<'_, AppState>,
    input: AssertBalanceInput,
) -> Result<AssertBalanceResult, IpcError> {
    assert_balance_impl(state.inner(), input)
}

/// The still-unexplained adjustment ("plug") for an account, or null if it has no
/// balance assertion (or it is fully explained → 0).
pub fn account_unexplained_impl(
    state: &AppState,
    account_id: String,
) -> Result<Option<MoneyDto>, IpcError> {
    let account = parse_account_id(&account_id)?;
    with_kernel(state, |kernel| {
        Ok(kernel.account_unexplained(account)?.map(MoneyDto::from))
    })
}

#[tauri::command]
#[specta::specta]
pub fn account_unexplained(
    state: tauri::State<'_, AppState>,
    account_id: String,
) -> Result<Option<MoneyDto>, IpcError> {
    account_unexplained_impl(state.inner(), account_id)
}

/// Convert an account's unexplained adjustment into one real transaction (ADR 0027
/// §8, personal-cfo-dyy4) — the plug goes to zero. Rejected if already explained.
pub fn convert_unexplained_to_transaction_impl(
    state: &AppState,
    account_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    with_kernel(state, |kernel| {
        let id = parse_account_id(&account_id)?;
        let outcome = kernel.dispatch(CommandEnvelope::new(
            user_meta(&idempotency_key),
            ConvertUnexplainedToTransaction::new(id),
        ))?;
        Ok(outcome.into())
    })
}

#[tauri::command]
#[specta::specta]
pub fn convert_unexplained_to_transaction(
    state: tauri::State<'_, AppState>,
    account_id: String,
    idempotency_key: String,
) -> Result<MutationResult, IpcError> {
    convert_unexplained_to_transaction_impl(state.inner(), account_id, idempotency_key)
}

// ---- unconfirmed_past_due (personal-cfo-4d8.27.7.6) ------------------------

/// Obligations whose scheduled date has passed with nothing recorded against them.
///
/// Confirmation is proved by `confirmed_obligations`, never the instance row's status
/// (ADR 0058 §2) — a far-early or $0 confirm leaves that status stale, and listing an
/// already-confirmed occurrence would invite a second, duplicate payment.
pub fn unconfirmed_past_due_impl(
    state: &AppState,
) -> Result<Vec<UnconfirmedOccurrenceDto>, IpcError> {
    with_kernel(state, |kernel| {
        Ok(kernel
            .unconfirmed_past_due()?
            .into_iter()
            .map(UnconfirmedOccurrenceDto::from)
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn unconfirmed_past_due(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<UnconfirmedOccurrenceDto>, IpcError> {
    unconfirmed_past_due_impl(state.inner())
}

// ---- connectors (personal-cfo-gglk, ADR 0060) ------------------------------
//
// The user-token connector tier made real: link stores the credential as a
// vault secret (ADR 0060 §1 — backup/restore round-trips the connection),
// sync rides the standard staged-ingestion pipeline with adapter provenance
// on source_batch/parser_run rows, and per-account since-watermarks advance
// only for windows the provider reported complete. Network calls run via
// spawn_blocking in the async wrappers and NEVER under the controller mutex.
//
// LEAK RULE: the credential is wrapped in `connector_core::Credential` the
// moment it leaves the DB and never appears in logs, errors, or DTOs.

use connector_core::{
    Connection as ProviderConnection, ConnectorAdapter, ConnectorError,
    Credential as ProviderCredential, LinkInput as ProviderLinkInput, LinkSession,
};
use tauri::Manager as _;

use crate::ipc::dto::{
    ConnectorAccountLinkDto, ConnectorConnectionDto, ConnectorExternalAccountDto,
    ConnectorForgetInput, ConnectorLinkInput, ConnectorLinkResultDto, ConnectorSetAccountLinkInput,
    ConnectorSyncInput, ConnectorSyncResultDto,
};

/// Auto-sync debounce: a connection synced (or attempted) within this many
/// hours is skipped on vault open — the daily-batch Bridge and its ~24
/// requests/day budget make anything more frequent pure waste (ADR 0060 §4).
const CONNECTOR_SYNC_DEBOUNCE_HOURS: i64 = 6;

/// Registry lookup shape, injected so tests drive the mock adapter.
pub type ConnectorResolver = fn(&str) -> Option<&'static dyn ConnectorAdapter>;

/// The active vault's path — captured before any connector network call as
/// the vault-identity witness.
fn current_vault_path(state: &AppState) -> Result<std::path::PathBuf, IpcError> {
    Ok(state.lock_controller()?.path().to_path_buf())
}

/// `with_kernel`, but ABORTS if the active vault is no longer `expected` —
/// an in-flight connector operation (network walks take minutes) must never
/// write another vault's data (personal-cfo-gglk review: cross-vault bleed
/// via switch_vault mid-walk).
fn with_vault_bound<T>(
    state: &AppState,
    expected: &std::path::Path,
    f: impl FnOnce(&Kernel) -> Result<T, IpcError>,
) -> Result<T, IpcError> {
    let guard = state.lock_controller()?;
    if guard.path() != expected {
        return Err(IpcError::Unavailable(
            "the active vault changed during the connector operation — nothing was written"
                .to_owned(),
        ));
    }
    let kernel = guard.kernel().ok_or(IpcError::VaultLocked)?;
    f(kernel)
}

fn parse_connector_connection_id(raw: &str) -> Result<Uuid, IpcError> {
    Uuid::parse_str(raw).map_err(|_| IpcError::Validation(format!("invalid connection id {raw:?}")))
}

/// Map a link-time adapter failure onto the IPC error surface. Messages come
/// from the adapter, which is leak-safe by contract (no URLs, sanitized).
fn connector_link_error(err: &ConnectorError) -> IpcError {
    match err {
        ConnectorError::NeedsUserAction { code, message, .. } => {
            IpcError::Validation(format!("{code}: {message}"))
        }
        other => IpcError::Unavailable(other.to_string()),
    }
}

pub fn connector_link_impl(
    state: &AppState,
    adapter: &dyn ConnectorAdapter,
    input: ConnectorLinkInput,
) -> Result<ConnectorLinkResultDto, IpcError> {
    // The witness is captured BEFORE the network claim: the credential must
    // land in the vault whose kernel initiated the link, never a vault the
    // user switched to mid-claim.
    let vault = current_vault_path(state)?;
    let session = adapter
        .link(&ProviderLinkInput {
            user_token: Some(ProviderCredential::new(input.setup_token)),
            params: std::collections::BTreeMap::new(),
        })
        .map_err(|e| connector_link_error(&e))?;
    let LinkSession::Established {
        credential,
        display_hint,
    } = session
    else {
        return Err(IpcError::Validation(
            "this adapter requires browser-based auth, which is not supported yet".to_owned(),
        ));
    };

    // Store FIRST: the setup token was single-use, so the claimed credential
    // must be durable before anything else can fail (a crash between claim
    // and store would waste the claim irrecoverably).
    let connection_id = Uuid::now_v7();
    with_vault_bound(state, &vault, |kernel| {
        kernel.create_connector_connection(
            connection_id,
            adapter.id(),
            credential.expose_secret(),
            display_hint.as_deref(),
        )?;
        Ok(())
    })?;

    // Best-effort account discovery (one request, outside the lock).
    let discovered = adapter.fetch_accounts(&ProviderConnection {
        credential: credential.clone(),
    });

    with_vault_bound(state, &vault, |kernel| {
        let mut accounts = Vec::new();
        let mut fetch_error = None;
        match &discovered {
            Ok(list) => {
                for acct in list {
                    let Some(external_id) = acct.external_id.as_deref() else {
                        continue;
                    };
                    kernel.upsert_connector_link(
                        connection_id,
                        external_id,
                        acct.external_name.as_deref(),
                    )?;
                    accounts.push(ConnectorExternalAccountDto {
                        external_id: external_id.to_owned(),
                        external_name: acct.external_name.clone(),
                    });
                }
            }
            Err(err) => fetch_error = Some(err.to_string()),
        }
        Ok(ConnectorLinkResultDto {
            connection_id: connection_id.to_string(),
            display_hint: display_hint.clone(),
            accounts,
            fetch_error,
        })
    })
}

#[tauri::command]
#[specta::specta]
pub async fn connector_link(
    app: tauri::AppHandle,
    input: ConnectorLinkInput,
) -> Result<ConnectorLinkResultDto, IpcError> {
    // Network in spawn_blocking: a stalled provider must not freeze the UI.
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let adapter = connector_core::connector_by_id(&input.adapter_id).ok_or_else(|| {
            IpcError::Validation(format!("no connector adapter {:?}", input.adapter_id))
        })?;
        connector_link_impl(&state, adapter, input)
    })
    .await
    .map_err(|_| IpcError::Unavailable("connector task failed".to_owned()))?
}

pub fn connector_connections_impl(
    state: &AppState,
) -> Result<Vec<ConnectorConnectionDto>, IpcError> {
    with_kernel(state, |kernel| {
        let mut out = Vec::new();
        for row in kernel.connector_connections()? {
            let links = kernel
                .connector_links(row.id)?
                .into_iter()
                .map(|l| ConnectorAccountLinkDto {
                    external_id: l.external_id,
                    external_name: l.external_name,
                    account_id: l.account_id.map(|a| a.to_string()),
                    last_synced_on: l.last_synced_on,
                })
                .collect();
            out.push(ConnectorConnectionDto {
                id: row.id.to_string(),
                adapter_id: row.adapter_id,
                display_hint: row.display_hint,
                last_synced_at: row.last_synced_at,
                last_error: row.last_error,
                links,
            });
        }
        Ok(out)
    })
}

#[tauri::command]
#[specta::specta]
pub fn connector_connections(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ConnectorConnectionDto>, IpcError> {
    connector_connections_impl(state.inner())
}

pub fn connector_set_account_link_impl(
    state: &AppState,
    input: ConnectorSetAccountLinkInput,
) -> Result<(), IpcError> {
    let connection_id = parse_connector_connection_id(&input.connection_id)?;
    let account = match input.account_id.as_deref() {
        Some(raw) => Some(parse_account_id(raw)?),
        None => None,
    };
    with_kernel(state, |kernel| {
        // A poisoned mapping (nonexistent account) would hard-fail every
        // future sync mid-batch — reject it here instead. Currency mismatch
        // is caught by the commit path for transactions and by the promotion
        // guard for balances (yl53); the link stores no currency to check.
        if let Some(account) = account {
            if !kernel.account_exists(account)? {
                return Err(IpcError::Validation("unknown account".to_owned()));
            }
        }
        kernel.set_connector_link_account(
            connection_id,
            &input.external_id,
            account.map(|a| a.as_uuid()),
        )?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn connector_set_account_link(
    state: tauri::State<'_, AppState>,
    input: ConnectorSetAccountLinkInput,
) -> Result<(), IpcError> {
    connector_set_account_link_impl(state.inner(), input)
}

/// Resolve the structured watermark-hold scopes (SyncBatch.held_account_ids,
/// raw provider ids) onto link keys — composite `conn/id` keys match by
/// suffix. Structured on purpose: this must never depend on warning prose.
fn held_link_ids(held_raw: &[String], links: &[finance_kernel::ConnectorLinkRow]) -> Vec<String> {
    let mut held = Vec::new();
    for raw in held_raw {
        for link in links {
            if link.external_id == *raw || link.external_id.ends_with(&format!("/{raw}")) {
                held.push(link.external_id.clone());
            }
        }
    }
    held
}

/// Releases the [`AppState`] in-flight claim on every exit path.
struct SyncClaim<'a> {
    state: &'a AppState,
    connection_id: Uuid,
}

impl Drop for SyncClaim<'_> {
    fn drop(&mut self) {
        self.state.release_connector_sync(self.connection_id);
    }
}

/// Classify a provider failure into a typed sync status and record it on the
/// connection's health row. Shared by the transaction walk and the unmapped
/// account-discovery pass (personal-cfo-k025) so both surface identically:
/// rate-limits stay healthy and consume the debounce; network errors record
/// without stamping (an offline unlock retries soon); everything else stamps
/// and records (retrying without the user helps nothing).
fn record_connector_failure(
    state: &AppState,
    vault: &std::path::Path,
    connection_id: Uuid,
    err: &ConnectorError,
) -> Result<(&'static str, String), IpcError> {
    let (status, message) = match err {
        // Healthy throttle (ADR 0060 §5): the connection stays
        // active and errorless; the debounce spaces the retry.
        ConnectorError::RateLimited(m) => ("rate_limited", m.clone()),
        ConnectorError::Expired(m) => ("expired", m.clone()),
        ConnectorError::NeedsUserAction { code, message, .. } => {
            ("needs_user_action", format!("{code}: {message}"))
        }
        other => ("failed", other.to_string()),
    };
    with_vault_bound(state, vault, |kernel| {
        match err {
            ConnectorError::RateLimited(_) => {
                kernel.record_connector_sync(connection_id, None)?;
            }
            ConnectorError::Network(m) => {
                kernel.record_connector_error(connection_id, m)?;
            }
            _ => {
                kernel.record_connector_sync(connection_id, Some(&message))?;
            }
        }
        Ok(())
    })?;
    Ok((status, message))
}

pub fn connector_sync_impl(
    state: &AppState,
    adapter: &dyn ConnectorAdapter,
    input: ConnectorSyncInput,
) -> Result<ConnectorSyncResultDto, IpcError> {
    let connection_id = parse_connector_connection_id(&input.connection_id)?;
    let empty = |status: &str, message: Option<String>| ConnectorSyncResultDto {
        connection_id: connection_id.to_string(),
        status: status.to_owned(),
        staged: 0,
        committed: 0,
        flagged: 0,
        skipped_unmapped: 0,
        warnings: Vec::new(),
        message,
    };

    // One sync per connection at a time: the unlock auto-sync, a manual sync,
    // and rapid re-unlocks must never walk the network concurrently.
    if !state.claim_connector_sync(connection_id) {
        return Ok(empty(
            "sync_in_progress",
            Some("a sync for this connection is already running".to_owned()),
        ));
    }
    let _claim = SyncClaim {
        state,
        connection_id,
    };

    // Phase 1 — read what the walk needs, then RELEASE the lock. Existence
    // is checked WITHOUT selecting the credential (no secret materializes on
    // an early exit), and the vault-identity witness is captured before any
    // network I/O.
    let vault = current_vault_path(state)?;
    let links = with_kernel(state, |kernel| {
        if !kernel.connector_connection_exists(connection_id)? {
            return Err(IpcError::Validation(
                "unknown connector connection".to_owned(),
            ));
        }
        Ok(kernel.connector_links(connection_id)?)
    })?;
    let account_map: std::collections::BTreeMap<String, Uuid> = links
        .iter()
        .filter_map(|l| l.account_id.map(|a| (l.external_id.clone(), a)))
        .collect();
    if account_map.is_empty() {
        // "Sync to discover them" must be TRUE before any mapping exists
        // (personal-cfo-k025): the real Bridge often has no account list
        // ready at link time, so an early return here strands the user in a
        // map-nothing/sync-nothing dead end. Do the cheap balances-only
        // discovery fetch, persist what it finds, and say so.
        let credential = with_kernel(state, |kernel| {
            kernel
                .connector_credential(connection_id)?
                .map(ProviderCredential::new)
                .ok_or_else(|| IpcError::Validation("unknown connector connection".to_owned()))
        })?;
        return match adapter.fetch_accounts(&ProviderConnection { credential }) {
            Ok(list) => {
                let discovered = with_vault_bound(state, &vault, |kernel| {
                    if !kernel.connector_connection_exists(connection_id)? {
                        return Err(IpcError::Validation(
                            "connection no longer exists — nothing was written".to_owned(),
                        ));
                    }
                    let mut count: u32 = 0;
                    for acct in &list {
                        if let Some(external_id) = acct.external_id.as_deref() {
                            kernel.upsert_connector_link(
                                connection_id,
                                external_id,
                                acct.external_name.as_deref(),
                            )?;
                            count += 1;
                        }
                    }
                    // Healthy discovery clears a stale error but does NOT
                    // stamp last_synced_at: the badge stays "Never synced"
                    // and the unlock auto-sync keeps re-discovering until an
                    // account is mapped.
                    kernel.clear_connector_error(connection_id)?;
                    Ok(count)
                })?;
                if discovered > 0 {
                    Ok(empty(
                        "discovered_accounts",
                        Some(format!(
                            "Found {discovered} account(s) — map them in Settings, then sync."
                        )),
                    ))
                } else {
                    Ok(empty(
                        "no_mapped_accounts",
                        Some(
                            "The provider reported no accounts yet — new connections can take a while to appear at the Bridge. Try again soon."
                                .to_owned(),
                        ),
                    ))
                }
            }
            Err(err) => {
                let (status, message) =
                    record_connector_failure(state, &vault, connection_id, &err)?;
                Ok(empty(status, Some(message)))
            }
        };
    }
    let credential = with_kernel(state, |kernel| {
        kernel
            .connector_credential(connection_id)?
            .map(ProviderCredential::new)
            .ok_or_else(|| IpcError::Validation("unknown connector connection".to_owned()))
    })?;
    // The walk's lower bound: the OLDEST mapped watermark, full history if any
    // mapped account has never synced. The adapter rewinds it further itself.
    let mut since: Option<NaiveDate> = None;
    let mut all_have_watermarks = true;
    for link in links.iter().filter(|l| l.account_id.is_some()) {
        match link
            .last_synced_on
            .as_deref()
            .and_then(|s| s.parse::<NaiveDate>().ok())
        {
            Some(date) => since = Some(since.map_or(date, |s: NaiveDate| s.min(date))),
            None => all_have_watermarks = false,
        }
    }
    if !all_have_watermarks {
        since = None;
    }

    // Phase 2 — the network walk, no lock held.
    let outcome = adapter.sync(&ProviderConnection { credential }, since);

    // Phase 3 — persist the outcome.
    match outcome {
        Ok(synced) => {
            let ingested = with_vault_bound(state, &vault, |kernel| {
                // Forgotten mid-sync (or any residual identity drift): abort
                // before writing rather than inserting orphan rows.
                if !kernel.connector_connection_exists(connection_id)? {
                    return Err(IpcError::Validation(
                        "connection no longer exists — nothing was written".to_owned(),
                    ));
                }
                let mut response_keys: std::collections::BTreeSet<String> =
                    std::collections::BTreeSet::new();
                for acct in &synced.batch.accounts {
                    if let Some(external_id) = acct.external_id.as_deref() {
                        response_keys.insert(external_id.to_owned());
                        kernel.upsert_connector_link(
                            connection_id,
                            external_id,
                            acct.external_name.as_deref(),
                        )?;
                    }
                }
                let source_name = format!("{} sync", adapter.display_name());
                let result = kernel.ingest_sync_batch(
                    adapter.id(),
                    &synced.adapter_version,
                    &source_name,
                    &synced.batch,
                    &account_map,
                    &user_meta(&input.idempotency_key),
                )?;
                // Watermarks advance ONLY for links that (a) were mapped when
                // the walk started (phase-1 map — a mid-sync mapping keeps
                // its NULL watermark and full-history walk), (b) appeared in
                // the provider response (an absent account keeps its old
                // watermark so its gap is re-walked), and (c) are not
                // retry-held (structured SyncBatch scopes, ADR 0060 §5).
                if !synced.hold_all_watermarks {
                    let held = held_link_ids(&synced.held_account_ids, &links);
                    let advance: Vec<String> = account_map
                        .keys()
                        .filter(|key| response_keys.contains(*key))
                        .filter(|key| !held.contains(*key))
                        .cloned()
                        .collect();
                    if !advance.is_empty() {
                        let synced_on = chrono::Utc::now().date_naive().to_string();
                        kernel.advance_connector_watermarks(connection_id, &synced_on, &advance)?;
                    }
                }
                kernel.record_connector_sync(connection_id, None)?;
                Ok(result)
            });
            let result = match ingested {
                Ok(result) => result,
                Err(err) => {
                    // Ingest failures (e.g. a poisoned mapping) must reach the
                    // health surface, not vanish into a tracing::warn loop.
                    let msg = format!("sync ingest failed: {err}");
                    let _ = with_vault_bound(state, &vault, |kernel| {
                        kernel.record_connector_error(connection_id, &msg)?;
                        Ok(())
                    });
                    return Err(err);
                }
            };
            let warnings: Vec<String> = synced
                .batch
                .warnings
                .iter()
                .map(|w| w.message.clone())
                .collect();
            let status = if result.batch.status == "committed" {
                "synced".to_owned()
            } else {
                result.batch.status.clone()
            };
            // The auto-categorization hook already runs inside
            // ingest_sync_batch (kz88 — same setting-gated merchant-memory
            // apply as file imports); say so when it did something.
            let message = (result.batch.auto_categorized > 0).then(|| {
                format!(
                    "auto-categorized {} transaction(s) from merchant memory",
                    result.batch.auto_categorized
                )
            });
            Ok(ConnectorSyncResultDto {
                connection_id: connection_id.to_string(),
                status,
                staged: result.batch.staged,
                committed: result.batch.committed,
                flagged: result.batch.flagged,
                skipped_unmapped: u32::try_from(result.skipped_unmapped).unwrap_or(u32::MAX),
                warnings,
                message,
            })
        }
        Err(err) => {
            let (status, message) = record_connector_failure(state, &vault, connection_id, &err)?;
            Ok(empty(status, Some(message)))
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn connector_sync(
    app: tauri::AppHandle,
    input: ConnectorSyncInput,
) -> Result<ConnectorSyncResultDto, IpcError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let adapter_id = with_kernel(&state, |kernel| {
            let connection_id = parse_connector_connection_id(&input.connection_id)?;
            kernel
                .connector_connections()?
                .into_iter()
                .find(|c| c.id == connection_id)
                .map(|c| c.adapter_id)
                .ok_or_else(|| IpcError::Validation("unknown connector connection".to_owned()))
        })?;
        let adapter = connector_core::connector_by_id(&adapter_id)
            .ok_or_else(|| IpcError::Unavailable(format!("no adapter {adapter_id:?}")))?;
        connector_sync_impl(&state, adapter, input)
    })
    .await
    .map_err(|_| IpcError::Unavailable("connector task failed".to_owned()))?
}

/// Sync every connection that is not freshly synced (the vault-open hook +
/// the manual "sync all" entry point). Best-effort per connection: one
/// failing connection never blocks the others.
pub fn connector_auto_sync_impl(
    state: &AppState,
    resolve: ConnectorResolver,
) -> Result<Vec<ConnectorSyncResultDto>, IpcError> {
    let connections = with_kernel(state, |kernel| Ok(kernel.connector_connections()?))?;
    let mut results = Vec::new();
    for connection in connections {
        let fresh = connection
            .last_synced_at
            .as_deref()
            .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
            .is_some_and(|t| {
                chrono::Utc::now().signed_duration_since(t.with_timezone(&chrono::Utc))
                    < chrono::Duration::hours(CONNECTOR_SYNC_DEBOUNCE_HOURS)
            });
        if fresh {
            results.push(ConnectorSyncResultDto {
                connection_id: connection.id.to_string(),
                status: "skipped_debounced".to_owned(),
                staged: 0,
                committed: 0,
                flagged: 0,
                skipped_unmapped: 0,
                warnings: Vec::new(),
                message: None,
            });
            continue;
        }
        let Some(adapter) = resolve(&connection.adapter_id) else {
            tracing::warn!(adapter = %connection.adapter_id, "no adapter for stored connection");
            continue;
        };
        let input = ConnectorSyncInput {
            connection_id: connection.id.to_string(),
            idempotency_key: format!("connector-auto-{}", Uuid::now_v7()),
        };
        match connector_sync_impl(state, adapter, input) {
            Ok(result) => results.push(result),
            Err(err) => tracing::warn!(error = %err, "connector auto-sync failed"),
        }
    }
    Ok(results)
}

#[tauri::command]
#[specta::specta]
pub async fn connector_auto_sync(
    app: tauri::AppHandle,
) -> Result<Vec<ConnectorSyncResultDto>, IpcError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        connector_auto_sync_impl(&state, connector_core::connector_by_id)
    })
    .await
    .map_err(|_| IpcError::Unavailable("connector task failed".to_owned()))?
}

pub fn connector_forget_impl(
    state: &AppState,
    input: ConnectorForgetInput,
) -> Result<(), IpcError> {
    let connection_id = parse_connector_connection_id(&input.connection_id)?;
    // No provider-side revoke call: the user-token tier has nothing to revoke
    // remotely (ConnectorAdapter::revoke is a documented no-op for SimpleFIN —
    // the user deletes the app at the Bridge); a real revoke lands with the
    // relay tier. Past synced ledger data deliberately survives.
    with_kernel(state, |kernel| {
        kernel.delete_connector_connection(connection_id)?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn connector_forget(
    state: tauri::State<'_, AppState>,
    input: ConnectorForgetInput,
) -> Result<(), IpcError> {
    connector_forget_impl(state.inner(), input)
}

#[cfg(test)]
mod connector_tests {
    use super::held_link_ids;
    use finance_kernel::ConnectorLinkRow;
    use uuid::Uuid;

    // The setup-token input must never regain Serialize: one
    // serde_json::to_string away from a leaked secret otherwise.
    static_assertions::assert_not_impl_any!(
        crate::ipc::dto::ConnectorLinkInput: serde::Serialize
    );

    fn link(external_id: &str) -> ConnectorLinkRow {
        ConnectorLinkRow {
            connection_id: Uuid::nil(),
            external_id: external_id.to_owned(),
            external_name: None,
            account_id: Some(Uuid::nil()),
            last_synced_on: None,
        }
    }

    #[test]
    fn held_scopes_match_raw_and_composite_link_keys() {
        let links = [link("C1/ACT-1"), link("ACT-2"), link("C2/ACT-1")];
        let held = held_link_ids(&["ACT-1".to_owned()], &links);
        assert_eq!(
            held,
            vec!["C1/ACT-1".to_owned(), "C2/ACT-1".to_owned()],
            "a raw provider id holds every connection-scoped key carrying it"
        );
        assert!(held_link_ids(&[], &links).is_empty());
        assert!(held_link_ids(&["ACT-9".to_owned()], &links).is_empty());
    }

    #[test]
    fn link_input_debug_redacts_the_setup_token() {
        let input = crate::ipc::dto::ConnectorLinkInput {
            adapter_id: "simplefin".to_owned(),
            setup_token: "CFO-CANARY-9f2d7c1e".to_owned(),
        };
        let debug = format!("{input:?}");
        assert!(!debug.contains("CFO-CANARY"), "token leaked: {debug}");
    }
}

#[cfg(test)]
mod household_timezone_tests {
    use super::resolve_initial_household_timezone;

    // personal-cfo-q329, ADR 0021 addendum: the machine zone is a fine INITIAL default at
    // vault creation (never the authoritative value — that's the stored column, changeable
    // any time via `set_household_timezone`), but only when the OS actually named a zone.
    #[test]
    fn a_successful_capture_is_used_as_is() {
        assert_eq!(
            resolve_initial_household_timezone(Ok("America/Chicago".to_owned())),
            Some("America/Chicago".to_owned()),
        );
    }

    #[test]
    fn a_failed_capture_falls_back_to_none_leaving_the_utc_default() {
        assert_eq!(
            resolve_initial_household_timezone(Err(iana_time_zone::GetTimezoneError::OsError)),
            None,
        );
    }
}

#[cfg(test)]
mod release_update_failure_tests {
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };

    use observability::RedactingMakeWriter;
    use tracing_subscriber::{fmt::MakeWriter, prelude::*};

    use super::record_release_update_failure_impl;
    use crate::ipc::dto::ReleaseUpdateFailureKind;

    #[derive(Clone, Default)]
    struct BufWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for BufWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("test log buffer lock")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for BufWriter {
        type Writer = BufWriter;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn updater_failure_tracing_is_structured_redacted_and_vault_independent() {
        let buffer = BufWriter::default();
        // Build the test-only sensitive value at runtime so the repository's
        // real-value scanner does not mistake its source text for user data.
        let sensitive_account = ["1234", "5678", "9012", "3456"].concat();
        let sensitive_error = format!("account {sensitive_account} could not install update");
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(RedactingMakeWriter::new(buffer.clone()));
        let subscriber = tracing_subscriber::registry().with(layer);

        // This invokes the implementation directly: it must not need AppState or vault access.
        tracing::subscriber::with_default(subscriber, || {
            record_release_update_failure_impl(
                "network error while downloading release",
                ReleaseUpdateFailureKind::Download,
            );
            record_release_update_failure_impl(&sensitive_error, ReleaseUpdateFailureKind::Install);
        });

        let logged = String::from_utf8(buffer.0.lock().expect("test log buffer lock").clone())
            .expect("test log output is UTF-8");

        for required_field in [
            "release updater failure recorded",
            "command",
            "record_release_update_failure",
            "command_id",
            "correlation_id",
            "causation_id",
            "actor_type",
            "actor_id",
            "failure_kind",
            "duration_ms",
            "outcome",
            "local-user",
            "none",
            "download",
            "install",
            "network error while downloading release",
        ] {
            assert!(
                logged.contains(required_field),
                "missing {required_field:?} in {logged}"
            );
        }
        assert!(
            logged.contains("[ACCT_NUMBER]"),
            "expected account redaction in {logged}"
        );
        assert!(
            !logged.contains(&sensitive_account),
            "raw account number leaked in {logged}"
        );
    }
}
