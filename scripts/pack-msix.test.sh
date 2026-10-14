#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Tests for the Windows packaging contract of the app:
#   .github/workflows/tauri-release.yml  ←→  scripts/pack-msix.ps1  ←→  the repo
#
# Run:  scripts/pack-msix.test.sh
#
# Why a shell test for a PowerShell script: the MSIX leg only ever runs on a
# Windows runner, behind the `vars.MSIX_IDENTITY_NAME` gate, which is still
# empty. That gate is why hub#577 went unnoticed for months — the packer wrote
# `erplora-bridge.msix` while the workflow uploaded `erplora-app.msix` with
# `if-no-files-found: error`, so the step could never have worked. Nothing
# executed it, so nothing said so.
#
# These tests do NOT run `winapp pack`. They check the things that made that
# bug possible and that are perfectly checkable off Windows: that the names,
# paths and tokens the workflow, the packer and the manifest exchange all agree
# with each other and with the files actually in the repo.
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PS1="$ROOT/scripts/pack-msix.ps1"
WORKFLOW="$ROOT/.github/workflows/tauri-release.yml"
TAURI_CONF="$ROOT/src-tauri/tauri.conf.json"
CARGO_TOML="$ROOT/src-tauri/Cargo.toml"

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

# ── Readers ──────────────────────────────────────────────────────────────────

# A scalar field of the $p preset hashtable, e.g. `OutName = "erplora-app.msix"`.
ps_field() {
    local matches
    matches=$(sed -n "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*\"\([^\"]*\)\".*/\1/p" "$PS1")
    printf '%s\n' "${matches%%$'\n'*}"
}

# The quoted entries of `ExeCandidates = @( ... )`, repo-relative.
exe_candidates() {
    sed -n '/ExeCandidates[[:space:]]*=[[:space:]]*@(/,/^[[:space:]]*)/p' "$PS1" \
        | grep -o '"[^"]*"' | tr -d '"' | sed 's|\$repoRoot/||'
}

# The asset file names the packer copies into the staged Assets/ directory.
staged_assets() {
    sed -n 's/^foreach (\$i in \(.*\)) {.*/\1/p' "$PS1" | grep -o '"[^"]*"' | tr -d '"'
}

# The __MSIX_*__ tokens the packer substitutes in the manifest.
replaced_tokens() {
    grep -o 'Replace("__MSIX_[A-Z_]*__"' "$PS1" | sed 's/Replace("//;s/"//' | sort -u
}

# The __MSIX_*__ tokens the manifest actually declares.
manifest_tokens() {
    grep -o '__MSIX_[A-Z_]*__' "$1" | sort -u
}

# `path:` of the workflow step that uploads the MSIX artifact.
msix_upload_path() {
    local paths
    paths=$(grep -A3 'name: app-msix' "$WORKFLOW" \
        | sed -n 's/^[[:space:]]*path:[[:space:]]*//p')
    printf '%s\n' "${paths%%$'\n'*}"
}

json_field() { python3 -c "import json,sys;print(json.load(open(sys.argv[1])).get(sys.argv[2],''))" "$1" "$2"; }

manifest_path="$ROOT/$(ps_field Manifest | sed 's|\$repoRoot/||')"
assets_dir="$ROOT/$(ps_field AssetsDir | sed 's|\$repoRoot/||')"
exe_name="$(ps_field ExeName)"
out_name="$(ps_field OutName)"

echo "MSIX packaging contract"

# ── 1. hub#577: the packer writes exactly the file the workflow uploads ───────
# The original defect in one line. `if-no-files-found: error` means any drift
# here fails the release on the first tag, not quietly.
upload_path="$(msix_upload_path)"
[ -n "$out_name" ] && [ "$upload_path" = "dist-msix/$out_name" ] \
    && ok "the workflow uploads the file the packer writes ($out_name)" \
    || bad "the workflow uploads the file the packer writes" \
           "packer OutName='$out_name' but the workflow uploads '$upload_path'"

# ── 2. hub#577: a single flavour, and the workflow calls it without one ───────
# The packer used to be `-Flavor bridge`-defaulted, so calling it with no
# flavour silently packaged the Bridge. With the Bridge retired there is one
# artefact to build; a reintroduced flavour parameter must not be defaulted.
# Matched on the variable, not the word: the tombstone comment that records the
# defect names `-Flavor` on purpose and is worth keeping.
! grep -q '\$Flavor' "$PS1" \
    && ok "the packer has no \$Flavor parameter to fall through (Bridge is gone)" \
    || bad "the packer has no \$Flavor parameter to fall through" \
           "\$Flavor is back in pack-msix.ps1 — a defaulted flavour is what hub#577 was"

# ── 3. hub#577 §1: the exe is found whatever Tauri decides to call it ─────────
# `productName` is "ERPlora" but the cargo bin is `erplora-tauri`; Tauri renames
# the binary after building. That rename has never been observed on a Windows
# runner here, so the packer must locate BOTH names — it stages the file as
# `ExeName` either way, so being right does not depend on knowing the answer.
product_name="$(json_field "$TAURI_CONF" productName)"
cargo_bins="$(sed -n 's/^name = "\(.*\)"/\1/p' "$CARGO_TOML")"
cargo_bin="${cargo_bins%%$'\n'*}"
candidates="$(exe_candidates)"
missing=""
for want in "$product_name.exe" "$cargo_bin.exe"; do
    grep -qF "/$want" <<<"$candidates" || missing="$missing $want"
done
[ -z "$missing" ] \
    && ok "the exe candidates cover both the productName and the cargo bin name" \
    || bad "the exe candidates cover both the productName and the cargo bin name" \
           "never looked for:$missing — candidates are:$(echo " $candidates" | tr '\n' ' ')"

# ── 4. Both target dirs: `tauri build` may run from the workspace root or from
#      src-tauri, and they do not share a target/ directory. ───────
for dir in "target/release" "src-tauri/target/release"; do
    grep -q "^$dir/" <<<"$candidates" \
        && ok "the exe is looked for in $dir/" \
        || bad "the exe is looked for in $dir/" "no candidate under $dir/"
done

# ── 5. The manifest launches the file the packer actually stages ─────────────
# The exe is staged under `ExeName`, and MSIX refuses to install if
# Application/@Executable names a file that is not in the package.
declared_exes="$(grep -o 'Executable="[^"]*"' "$manifest_path")"
declared_exe="${declared_exes%%$'\n'*}"
declared_exe="${declared_exe#Executable=\"}"
declared_exe="${declared_exe%\"}"
[ -n "$exe_name" ] && [ "$declared_exe" = "$exe_name" ] \
    && ok "the manifest launches the staged exe ($exe_name)" \
    || bad "the manifest launches the staged exe" \
           "manifest Executable='$declared_exe' but the packer stages '$exe_name'"

# ── 6. Every asset the packer copies exists, at the path it copies it from ───
for a in $(staged_assets); do
    [ -f "$assets_dir/$a" ] \
        && ok "asset present: $a" \
        || bad "asset present: $a" "not found at $assets_dir/$a"
done

# ── 7. Tokens: the packer and the manifest agree, in both directions ─────────
# A token in the manifest that the packer does not replace ships a literal
# `__MSIX_…__` into the Store submission; the reverse is a silent no-op.
diff_out="$(diff <(replaced_tokens) <(manifest_tokens "$manifest_path") 2>&1)"
[ -z "$diff_out" ] \
    && ok "the packer replaces exactly the tokens the manifest declares" \
    || bad "the packer replaces exactly the tokens the manifest declares" \
           "$(echo "$diff_out" | tr '\n' ' ')"

# ── 8. hub#465 + hub#878: the release installs the CLI, never a frontend ─────
# hub#465 removed every JS step from this workflow on the grounds that nothing
# needed one: `tauri-action` only builds a frontend when tauri.conf.json has a
# `beforeBuildCommand`, and since ADR-0154 it has none. hub#878 falsified the
# other half of that reasoning — with no `@tauri-apps/cli` dependency the action
# falls back to `npm install -g`, which needs a writable /usr/lib/node_modules
# and so only ever worked on GitHub-hosted runners. The Linux job of v1.1.0 died
# there with EACCES.
#
# So a JS install is now expected, and what this asserts is that it stays the
# CHEAP one: the CLI only (`--filter @erplora/app`), never the web app's
# dependency tree, in the most expensive workflow we have. The `beforeBuildCommand`
# half stands: bring one back and the front has to be built somewhere.
has_before_build="$(python3 -c "import json;print('yes' if 'beforeBuildCommand' in json.load(open('$TAURI_CONF')).get('build',{}) else 'no')")"
install_steps="$(grep -c 'pnpm install --frozen-lockfile' "$WORKFLOW" || true)"
unfiltered="$(grep 'pnpm install --frozen-lockfile' "$WORKFLOW" | grep -vc -- '--filter @erplora/app' || true)"

[ "$install_steps" -ge 1 ] \
    && ok "the release installs the Tauri CLI from the workspace" \
    || bad "the release installs the Tauri CLI from the workspace" \
           "no \`pnpm install --frozen-lockfile\` left: tauri-action would go back to \`npm install -g\` (hub#878)"

if [ "$has_before_build" = no ]; then
    [ "$unfiltered" -eq 0 ] \
        && ok "no beforeBuildCommand ⇒ the release resolves the CLI only, not the web app" \
        || bad "no beforeBuildCommand ⇒ the release resolves the CLI only, not the web app" \
               "an unfiltered \`pnpm install\` resolves the whole workspace for a build that packs \`shell-dist/\`"
else
    [ "$unfiltered" -ge 1 ] \
        && ok "beforeBuildCommand present ⇒ the release installs what builds the frontend" \
        || bad "beforeBuildCommand present ⇒ the release installs what builds the frontend" \
               "tauri.conf.json builds a frontend, but only the CLI is installed — the build fails on the first tag"
fi

# ── 9. hub#878: the CLI is pinned, and the same one on the three platforms ───
# `npm install -g @tauri-apps/cli@v2` also meant every release was built with
# whatever the CLI was that morning, and possibly a different one per platform.
# A lockfile entry is what makes two builds of the same commit comparable.
cli_pin="$(python3 -c "import json;print(json.load(open('$ROOT/package.json'))['devDependencies']['@tauri-apps/cli'])" 2>/dev/null || true)"
case "$cli_pin" in
    [0-9]*) ok "the Tauri CLI is pinned to an exact version ($cli_pin)" ;;
    '')     bad "the Tauri CLI is pinned to an exact version" \
                "package.json does not declare @tauri-apps/cli — the global install comes back (hub#878)" ;;
    *)      bad "the Tauri CLI is pinned to an exact version" \
                "'$cli_pin' is a range: two builds of the same commit can use different CLIs" ;;
esac

grep -q 'tauriScript: pnpm exec tauri' "$WORKFLOW" \
    && ok "tauri-action runs the workspace CLI (\`tauriScript\`)" \
    || bad "tauri-action runs the workspace CLI (\`tauriScript\`)" \
           "without \`tauriScript\` the action ignores what we installed and calls \`npm install -g\` (hub#878)"

echo
printf 'passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
