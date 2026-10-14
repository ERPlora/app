//! Guard on the Windows application manifest reaching EVERY executable of this crate (hub#2705).
//!
//! `tauri-build` embeds its default manifest — the dependency on Common Controls v6 — through
//! `tauri-winres`, which links it with `cargo:rustc-link-arg-bins`: the APP binary gets it, the
//! test harnesses do not. Tauri's menu crate (`muda`) imports `TaskDialogIndirect` from
//! `comctl32.dll`, and that function only exists in v6. Without the manifest Windows loads the v5
//! in System32, the loader cannot resolve the import, and every test binary of this crate dies
//! before `main` with `0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND` — seen on the first Windows run of
//! `test-shell.yml`. The release job never noticed: it builds the app, it never runs a test exe.
//!
//! The fix is Tauri's documented one (`WindowsAttributes::new_without_app_manifest`): the build
//! script embeds the same manifest itself with `cargo:rustc-link-arg`, which applies to every
//! target of the package — the app, the lib's test harness and each file in `tests/`. These
//! assertions run on all three systems; the Windows job of `test-shell.yml` is where a missing
//! manifest would also show as the loader error above.

use std::path::PathBuf;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = crate_dir().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The build script's code, without comment lines: a comment naming the flag is not the flag.
fn build_script_code() -> String {
    read("build.rs")
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

const MANIFEST: &str = "windows-app-manifest.xml";

#[test]
fn the_manifest_declares_common_controls_v6() {
    let manifest = read(MANIFEST);
    assert!(
        manifest.contains("Microsoft.Windows.Common-Controls") && manifest.contains("6.0.0.0"),
        "{MANIFEST} must declare Common Controls 6.0.0.0: `TaskDialogIndirect` only exists in v6, \
         and without it every executable of the crate fails to load on Windows (hub#2705)",
    );
}

#[test]
fn the_build_script_embeds_it_in_every_target_not_only_the_app() {
    let code = build_script_code();
    assert!(
        code.contains("new_without_app_manifest"),
        "build.rs must turn off tauri-build's own manifest: it only reaches the app binary, and \
         embedding ours as well would put two manifests in the app (duplicate resource)",
    );
    assert!(
        code.contains("cargo:rustc-link-arg=/MANIFEST:EMBED"),
        "build.rs must embed the manifest with `cargo:rustc-link-arg` (every target), not \
         `rustc-link-arg-bins`: the test harnesses are the executables that were missing it",
    );
    assert!(
        code.contains("cargo:rustc-link-arg=/MANIFESTINPUT:") && code.contains(MANIFEST),
        "build.rs must hand the linker {MANIFEST} as the manifest input",
    );
    assert!(
        code.contains("cargo:rerun-if-changed="),
        "build.rs must re-run when {MANIFEST} changes, or an edit would not reach the binaries",
    );
}
