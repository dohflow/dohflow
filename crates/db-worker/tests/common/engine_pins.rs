//! The engine pin's file format and verdicts (personal-cfo-32fmp.4; vault-format.md C.13).
//!
//! `tests/engine_pins.txt` is append-only history: one section per engine generation, each holding,
//! per `WriteCommand` variant, an INPUT digest (the payload bytes and the replicated and artifact
//! state the command was applied on) and an OUTPUT digest (the outcome, the write-set hash and every
//! row the command changed), plus the outcome of every deterministic-rejection scenario.
//!
//! A change to what apply writes is the case where a variant's output changed while its input did
//! not. That requires a new engine generation: bump `ENGINE_VERSION` and append a section. A change
//! only to the input (a sample or its setup changed for another reason, or an upstream variant's
//! output moved the base) re-pins the current section in place. Adding or removing a variant also
//! requires a bump: what a receiver accepts is part of the engine (C.13).

use std::collections::BTreeMap;

/// One generation's pin.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Section {
    pub engine: String,
    /// variant -> (input digest, output digest), hex.
    pub variants: BTreeMap<String, (String, String)>,
    /// scenario label -> outcome text.
    pub rejections: BTreeMap<String, String>,
}

pub fn parse(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(engine) = line.strip_prefix("engine ") {
            sections.push(Section {
                engine: engine.to_owned(),
                ..Section::default()
            });
            continue;
        }
        let section = sections
            .last_mut()
            .expect("a line before any `engine` line");
        if let Some(rest) = line.strip_prefix("variant ") {
            let mut parts = rest.split(' ');
            let (name, input, output) = (
                parts.next().unwrap(),
                parts.next().unwrap(),
                parts.next().unwrap(),
            );
            section
                .variants
                .insert(name.to_owned(), (input.to_owned(), output.to_owned()));
        } else if let Some(rest) = line.strip_prefix("rejection ") {
            let (label, outcome) = rest
                .split_once('\t')
                .expect("a tab between label and outcome");
            section
                .rejections
                .insert(label.to_owned(), outcome.to_owned());
        } else {
            panic!("unrecognized line in engine_pins.txt: {line}");
        }
    }
    sections
}

pub fn render(sections: &[Section]) -> String {
    let mut out = String::from(
        "# Engine pins (personal-cfo-32fmp.4, vault-format.md C.13). APPEND-ONLY history: a change to\n\
         # what apply writes bumps ENGINE_VERSION and appends a section; never edit an older one.\n\
         # Regenerate the current section with ENGINE_PIN_UPDATE=1 cargo test -p db-worker --test engine_pin\n",
    );
    for section in sections {
        out.push_str(&format!("\nengine {}\n", section.engine));
        for (name, (input, output)) in &section.variants {
            out.push_str(&format!("variant {name} {input} {output}\n"));
        }
        for (label, outcome) in &section.rejections {
            out.push_str(&format!("rejection {label}\t{outcome}\n"));
        }
    }
    out
}

/// The generation number N of `dohflow-engine-N`.
pub fn generation(engine: &str) -> u32 {
    engine
        .strip_prefix("dohflow-engine-")
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{engine} is not dohflow-engine-N"))
}

/// What the difference between a pinned section and the current output means.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing changed.
    Same,
    /// Only inputs changed: re-pin the section in place.
    Repin(Vec<String>),
    /// An output changed on an unchanged input, or a variant was added or removed: bump.
    Bump(Vec<String>),
}

pub fn judge(pinned: &Section, now: &Section) -> Verdict {
    let mut bump = Vec::new();
    let mut repin = Vec::new();
    for name in pinned.variants.keys() {
        if !now.variants.contains_key(name) {
            bump.push(format!("variant {name} was removed"));
        }
    }
    for (name, (input, output)) in &now.variants {
        match pinned.variants.get(name) {
            None => bump.push(format!("variant {name} was added")),
            Some((pinned_input, pinned_output)) => {
                if pinned_input == input && pinned_output != output {
                    bump.push(format!(
                        "{name}: apply wrote different rows for an unchanged payload and base"
                    ));
                } else if pinned_input != input {
                    repin.push(format!("{name}: its input (payload or base state) changed"));
                }
            }
        }
    }
    // Rejections are consensus output too (Addendum D rule 10). The same scenario with another
    // outcome is an engine change; a different scenario list re-pins.
    for (label, outcome) in &now.rejections {
        match pinned.rejections.get(label) {
            Some(pinned_outcome) if pinned_outcome != outcome => {
                bump.push(format!(
                    "rejection scenario `{label}`: {pinned_outcome} -> {outcome}"
                ));
            }
            None => repin.push(format!("rejection scenario `{label}` is new")),
            _ => {}
        }
    }
    for label in pinned.rejections.keys() {
        if !now.rejections.contains_key(label) {
            repin.push(format!("rejection scenario `{label}` is gone"));
        }
    }
    if !bump.is_empty() {
        Verdict::Bump(bump)
    } else if !repin.is_empty() {
        Verdict::Repin(repin)
    } else {
        Verdict::Same
    }
}
