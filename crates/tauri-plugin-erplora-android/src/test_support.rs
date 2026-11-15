//! What the tests of every module read: the Kotlin and build sources they hold the Rust side to,
//! and the helpers that read those sources as code rather than prose.

pub(crate) const PLUGIN_KT: &str = include_str!("../android/src/main/java/com/erplora/android/ErploraAndroidPlugin.kt");
pub(crate) const ERPLORA_ANDROID_PLUGIN_KT: &str = PLUGIN_KT;
pub(crate) const BUILD_RS: &str = include_str!("../build.rs");
pub(crate) const LIB_RS: &str = include_str!("lib.rs");
pub(crate) const DEFAULT_PERMISSIONS: &str = include_str!("../permissions/default.toml");

/// The plugin's Android library manifest, read as the merger reads it.
pub(crate) const PLUGIN_MANIFEST: &str = include_str!("../android/src/main/AndroidManifest.xml");

/// The Kotlin policy is the only place that can actually ASK for these permissions, and the
/// map it returns is keyed by the raw string. If the two sides ever spelled one differently,
/// Rust would read `None` for a permission Android had denied and the till would go back to
/// reporting zero printers with a straight face.
pub(crate) const PERMISSION_POLICY_KT: &str =
    include_str!("../android/src/main/java/com/erplora/android/PermissionPolicy.kt");

/// Everything outside XML comments. Commenting a tag out is the easiest way to "temporarily"
/// drop a permission, and the edit nobody remembers to undo.
pub(crate) fn without_comments(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find("-->") else { return out };
        rest = &rest[start + end + "-->".len()..];
    }
    out.push_str(rest);
    out
}

/// Permissions actually DECLARED by a manifest, read out of its live `<uses-permission>` tags.
///
/// Not a text search: manifests here carry long comments naming the permissions and explaining
/// why they are there, so `contains()` would keep passing on the prose after the tag itself was
/// deleted — a guard that goes green on the wreckage is worse than no guard.
pub(crate) fn declared_permissions(manifest: &str) -> Vec<String> {
    without_comments(manifest)
        .split("<uses-permission")
        .skip(1)
        .filter_map(|tag| {
            tag.split("android:name=\"")
                .nth(1)
                .and_then(|value| value.split('"').next())
                .map(str::to_owned)
        })
        .collect()
}

#[test]
fn a_commented_out_permission_does_not_count_as_declared() {
    // How a permission usually disappears: someone comments it out "for a minute". The prose
    // around it still names it, so a text search would call it declared and let the manifest
    // guards wave the regression through.
    let manifest = r#"<manifest>
            <uses-permission android:name="android.permission.INTERNET" />
            <!-- <uses-permission android:name="android.permission.ACCESS_LOCAL_NETWORK" /> -->
        </manifest>"#;

    assert_eq!(declared_permissions(manifest), ["android.permission.INTERNET"]);
}

/// Kotlin source with its comments taken out.
///
/// `DownloadPublisher` EXPLAINS why it does not use `WRITE_EXTERNAL_STORAGE`, and a plain
/// `contains` cannot tell an explanation from a call. Same reason as [`without_comments`] one
/// screen up: a guard has to read the code, not the documentation about the code.
pub(crate) fn without_kotlin_comments(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    loop {
        let block = rest.find("/*");
        let line = rest.find("//");
        let (start, close, skip) = match (block, line) {
            (Some(b), Some(l)) if b < l => (b, "*/", 2),
            (Some(b), None) => (b, "*/", 2),
            (_, Some(l)) => (l, "\n", 0),
            (None, None) => break,
        };
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find(close) else {
            break;
        };
        rest = &rest[start + end + skip..];
    }
    out.push_str(rest);
    out
}

#[test]
fn the_comment_stripper_keeps_the_code_and_drops_the_prose() {
    let source = "// asks for WRITE_EXTERNAL_STORAGE one day\nval a = 1\n/* WRITE_EXTERNAL_STORAGE */\nval b = 2";
    let code = without_kotlin_comments(source);
    assert!(!code.contains("WRITE_EXTERNAL_STORAGE"), "{code:?}");
    assert!(
        code.contains("val a = 1") && code.contains("val b = 2"),
        "{code:?}"
    );
}

/// The crate's Rust outside its tests: `lib.rs` and every module it declares, each up to its
/// `#[cfg(test)]`. The modules are read from `lib.rs` itself, so a new one is never left out.
pub(crate) fn production_rust() -> String {
    let code = |source: &str| source.split("#[cfg(test)]").next().unwrap_or_default().to_owned();
    let mut rust = code(LIB_RS);
    let modules: Vec<String> = rust
        .lines()
        .filter_map(|line| Some(line.strip_prefix("mod ")?.strip_suffix(';')?.to_owned()))
        .collect();
    for module in modules {
        let path = format!("{}/src/{module}.rs", env!("CARGO_MANIFEST_DIR"));
        let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        rust.push_str(&code(&source));
    }
    rust
}
