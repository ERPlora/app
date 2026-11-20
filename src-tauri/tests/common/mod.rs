//! What the guards on `MainActivity`'s wiring share: reading a file of the repository and reading
//! Kotlin the way the compiler does, without its comments.

use std::path::PathBuf;

pub const MAIN_ACTIVITY: &str =
    "src-tauri/gen/android/app/src/main/java/com/erplora/app/MainActivity.kt";

fn repo_root() -> PathBuf {
    // `src-tauri` → up one.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("cannot resolve the repository root")
}

pub fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// A Kotlin file without its comments: a commented-out call is not the call. Block comments nest
/// in Kotlin, and their lines need not start with `*`.
pub fn code_of(source: &str) -> String {
    let mut code = String::new();
    let mut rest = source;
    let mut depth = 0;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("/*") {
            depth += 1;
            rest = &rest[2..];
        } else if depth > 0 && rest.starts_with("*/") {
            depth -= 1;
            rest = &rest[2..];
        } else if depth == 0 && rest.starts_with("//") {
            rest = rest.find('\n').map_or("", |end| &rest[end..]);
        } else {
            if depth == 0 {
                code.push(c);
            }
            rest = &rest[c.len_utf8()..];
        }
    }
    code
}

/// What follows `signature` in `code`, up to the next function of the class: a call moved to a
/// function nobody calls does not count. `None` when there is no such function.
pub fn function_body<'a>(code: &'a str, signature: &str) -> Option<&'a str> {
    let start = code.find(signature)?;
    let rest = &code[start + signature.len()..];
    Some(rest.find("fun ").map_or(rest, |next| &rest[..next]))
}
