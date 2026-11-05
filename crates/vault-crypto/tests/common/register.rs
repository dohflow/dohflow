//! An in-memory [`RegisterStore`] for tests, shared between clones the way a reopened database is
//! shared between connections, with scripted failures. It passes the same conformance suite the
//! SQLite store of the snapshot-publish bead must pass.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use platform_traits::CounterContext;
use vault_crypto::sync_counter::{RegisterStore, RegisterStoreError, SavedSealed};

#[derive(Default)]
struct State {
    next: HashMap<CounterContext, u64>,
    saved: HashMap<CounterContext, SavedSealed>,
    fail_commits: usize,
    fail_set_saved: usize,
    commits: usize,
}

#[derive(Clone, Default)]
pub struct MemoryRegisterStore(Arc<Mutex<State>>);

impl MemoryRegisterStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next `count` advances fail with `Unavailable` before anything is written.
    pub fn fail_next_commits(&self, count: usize) {
        self.0.lock().unwrap().fail_commits = count;
    }

    /// The next `count` saved-record writes fail with `Unavailable`.
    pub fn fail_next_set_saved(&self, count: usize) {
        self.0.lock().unwrap().fail_set_saved = count;
    }

    /// How many advances succeeded.
    pub fn commits(&self) -> usize {
        self.0.lock().unwrap().commits
    }

    /// Roll a context's register back (what a restore of the vault does), bypassing the CAS.
    pub fn force_next(&self, context: &CounterContext, value: Option<u64>) {
        let mut state = self.0.lock().unwrap();
        match value {
            Some(v) => state.next.insert(*context, v),
            None => state.next.remove(context),
        };
    }
}

impl RegisterStore for MemoryRegisterStore {
    fn next_counter(&self, context: &CounterContext) -> Result<Option<u64>, RegisterStoreError> {
        Ok(self.0.lock().unwrap().next.get(context).copied())
    }

    fn commit_advance(
        &self,
        context: &CounterContext,
        from: Option<u64>,
        to: u64,
    ) -> Result<(), RegisterStoreError> {
        if to == 0 || from.is_some_and(|f| to <= f) {
            return Err(RegisterStoreError::BadAdvance);
        }
        let mut state = self.0.lock().unwrap();
        if state.fail_commits > 0 {
            state.fail_commits -= 1;
            return Err(RegisterStoreError::Unavailable);
        }
        if state.next.get(context).copied() != from {
            return Err(RegisterStoreError::Conflict);
        }
        state.next.insert(*context, to);
        state.commits += 1;
        Ok(())
    }

    fn saved(&self, context: &CounterContext) -> Result<Option<SavedSealed>, RegisterStoreError> {
        Ok(self.0.lock().unwrap().saved.get(context).copied())
    }

    fn set_saved(
        &self,
        context: &CounterContext,
        saved: Option<&SavedSealed>,
    ) -> Result<(), RegisterStoreError> {
        let mut state = self.0.lock().unwrap();
        if state.fail_set_saved > 0 {
            state.fail_set_saved -= 1;
            return Err(RegisterStoreError::Unavailable);
        }
        match saved {
            Some(record) => state.saved.insert(*context, *record),
            None => state.saved.remove(context),
        };
        Ok(())
    }
}
