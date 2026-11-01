//! A local, dev-only check (personal-cfo-5ie.13; never part of the shipped app): how many base
//! events did an earlier scenario apply wrongly retire?
//!
//! Before ADR 0055 section 2 was amended, applying a scenario with one one-off retired every base
//! one-off. This counts what is still retired, per adding kind, so the owner can tell whether any
//! data needs repair. It prints COUNTS ONLY: no amount, name, id, path or log line.
//!
//! It never touches the original vault. It copies the vault file (and its key envelope) into a
//! temporary directory, opens the COPY (which may migrate the copy to this build's schema), counts,
//! and lets the temporary directory go. Quit DohFlow first so the copy is consistent.
//!
//! ```text
//! cargo run -p finance-kernel --example count_wrongly_superseded -- /path/to/vault.db
//! ```
//!
//! The password is read at the terminal without echo and is never an argument or a file.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use finance_kernel::{Kernel, KernelError};

/// Appends `suffix` to the file name (`vault.db` + `.envelope` is `vault.db.envelope`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

/// Reads one line from the terminal with echo off. `None` unless stdin is a terminal.
fn read_password() -> Option<String> {
    if !std::io::stdin().is_terminal() {
        return None;
    }
    eprint!("Vault password: ");
    std::io::stderr().flush().ok()?;
    let echo = |on: bool| {
        Command::new("stty")
            .arg(if on { "echo" } else { "-echo" })
            .status()
            .is_ok_and(|status| status.success())
    };
    if !echo(false) {
        return None;
    }
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    echo(true);
    eprintln!();
    read.ok()?;
    Some(line.trim_end_matches(['\r', '\n']).to_owned())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [vault] = args.as_slice() else {
        eprintln!("usage: count_wrongly_superseded <path to the vault file>");
        return ExitCode::from(2);
    };
    let original = PathBuf::from(vault);

    let Some(password) = read_password() else {
        eprintln!("This needs a terminal to ask for the password.");
        return ExitCode::from(2);
    };

    let Ok(scratch) = tempfile::tempdir() else {
        eprintln!("Could not make a temporary folder.");
        return ExitCode::FAILURE;
    };
    let copy = scratch.path().join("vault.db");
    // The database, its key envelope, and its write-ahead files when the app left them behind.
    let mut copied = 0;
    for suffix in ["", ".envelope", "-wal", "-shm"] {
        let from = with_suffix(&original, suffix);
        if from.exists() {
            if std::fs::copy(&from, with_suffix(&copy, suffix)).is_err() {
                eprintln!("Could not copy the vault.");
                return ExitCode::FAILURE;
            }
            copied += 1;
        }
    }
    if copied < 2 {
        eprintln!("No vault found at that path.");
        return ExitCode::FAILURE;
    }

    let kernel = match Kernel::unlock_vault(&copy, password.as_bytes()) {
        Ok(kernel) => kernel,
        Err(KernelError::VaultUnlockFailed) => {
            eprintln!("The password was not accepted.");
            return ExitCode::FAILURE;
        }
        Err(KernelError::NewerVaultSchema { .. }) => {
            eprintln!("This vault is newer than this build; use a current build.");
            return ExitCode::FAILURE;
        }
        Err(_) => {
            eprintln!("The copy of the vault could not be opened.");
            return ExitCode::FAILURE;
        }
    };
    let Ok(counts) = kernel.count_events_wrongly_superseded_by_apply() else {
        eprintln!("The check could not be run.");
        return ExitCode::FAILURE;
    };
    let mut total = 0;
    for (kind, count) in counts {
        println!("{}: {count}", kind.as_token());
        total += count;
    }
    println!("total: {total}");
    ExitCode::SUCCESS
}
