//! Keys are built in place (personal-cfo-xzpra): no vault-crypto code outside the `SecretBytes`
//! module itself may construct a secret from an array it holds, because `SecretBytes::new` takes
//! the key by value and leaves a copy on the stack that nothing scrubs. Production code builds
//! keys with `SecretBytes::init` / `SecretBytes::from_slice`, which hand the closure the protected
//! buffer. A new `SecretBytes::new(` in `src/` (other than the module that defines it) fails here,
//! so the next key type cannot quietly reintroduce the copy.

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_key_is_built_from_a_stack_array_outside_the_secret_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.len() >= 5, "found {} source files", files.len());
    let mut hits = Vec::new();
    for file in files {
        if file.file_name().and_then(|n| n.to_str()) == Some("secret.rs") {
            continue;
        }
        for (number, line) in fs::read_to_string(&file).unwrap().lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if code.contains("SecretBytes::new(") || code.contains("::from_array(") {
                hits.push(format!(
                    "{}:{}: {}",
                    file.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "build keys in place with SecretBytes::init or from_slice:\n{}",
        hits.join("\n")
    );
}
