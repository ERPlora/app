//! The release gate: what makes a green tag mean "published" (hub#895).
//!
//! `tauri-release.yml` gates each publishing job behind a repository Variable, so an empty
//! Variable turns the job `skipped` — and a skipped job does not colour a run. Three tags in a row
//! finished GREEN having published nothing (the last one v1.0.3, hub#839): the run that shipped
//! everything and the run that shipped nothing looked identical from the outside.
//!
//! The fix is a final `release-gate` job that reads the result of every other job and fails when a
//! channel that was expected to publish did not. The logic lives in `scripts/release-gate.sh` and
//! is exercised by `scripts/release-gate.test.sh`; what CANNOT be tested there is the wiring, and
//! the wiring is where this class of bug hides:
//!
//! * a gate without `always()` is itself skipped the moment anything upstream fails or skips —
//!   the guard disappears exactly when it is needed;
//! * a gate that does not `needs:` a publishing job cannot see it, so adding a fourth channel
//!   tomorrow would silently reopen hub#895 for that channel only;
//! * a job in `needs:` whose result is never handed to the script is in the dependency graph but
//!   invisible to the decision — which reads as "checked" and is not.
//!
//! These assertions are the wiring. They run wherever `cargo test -p erplora-tauri` runs, and
//! `test-shell.yml` triggers on changes to `tauri-release.yml`, so they see the edit that would
//! break them before it merges — rather than on a tag, which cannot be re-run and is public by the
//! time anyone looks.

use std::path::PathBuf;

/// The id of the final job.
const GATE_JOB: &str = "release-gate";

/// The script the gate delegates its decision to.
const GATE_SCRIPT: &str = "scripts/release-gate.sh";

/// What the `version` job answers when this run is a release (scripts/release-version.sh).
const RELEASE_ONLY: &str = "needs.version.outputs.release == 'true'";

/// The job that creates the tag `v<version>` and the GitHub Release: the LAST step of a release.
const TAG_JOB: &str = "github-release";

fn repo_root() -> PathBuf {
    // `src-tauri` → up one.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("cannot resolve the repository root")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The release workflow with every comment line removed: a `#` line explaining a rule is not the
/// rule.
fn workflow_code() -> String {
    read(".github/workflows/tauri-release.yml")
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The job ids of the workflow, in file order: the two-space-indented keys under `jobs:`.
fn job_ids(code: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut inside_jobs = false;
    for line in code.lines() {
        if line.trim_end() == "jobs:" {
            inside_jobs = true;
            continue;
        }
        if !inside_jobs {
            continue;
        }
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        if rest.starts_with(' ') {
            continue;
        }
        if let Some(id) = rest.strip_suffix(':') {
            if !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

/// Everything a job declares, up to where the next job starts.
fn job_block(code: &str, id: &str) -> Option<String> {
    let start = code.find(&format!("\n  {id}:\n"))?;
    let tail = &code[start + 1..];
    let mut block = String::new();
    for (i, line) in tail.lines().enumerate() {
        // A new job starts at a two-space key that is not this job's own header line.
        if i > 0 && line.starts_with("  ") && !line.starts_with("   ") && line.ends_with(':') {
            break;
        }
        block.push_str(line);
        block.push('\n');
    }
    Some(block)
}

/// The `needs:` line of a job block, or "" when it has none.
fn needs_line(block: &str) -> String {
    block
        .lines()
        .find(|l| l.trim_start().starts_with("needs:"))
        .unwrap_or("")
        .to_string()
}

/// Everything the `release-gate:` job declares, up to where the next job starts.
fn gate_block(code: &str) -> String {
    job_block(code, GATE_JOB).unwrap_or_else(|| {
        panic!(
            "`tauri-release.yml` has no `{GATE_JOB}:` job. Without it a release whose publishing \
                 jobs were all skipped finishes GREEN and nobody learns that it shipped \
                 nothing — which happened three times, last with v1.0.3 (hub#895/hub#839)"
        )
    })
}

#[test]
fn the_release_ends_in_a_gate_that_runs_even_when_everything_else_did_not() {
    let block = gate_block(&workflow_code());

    // `always()` is the whole point: without it the gate inherits the default "only if every
    // `needs` succeeded", so a skipped publish job skips the gate too and the run is green again.
    // The guard would vanish in precisely the situation it was written for.
    assert!(
        block.contains("always()"),
        "the `{GATE_JOB}` job no longer runs with `always()`. A gate that only runs when its \
         dependencies succeeded cannot report a dependency that did not: the skipped publish jobs \
         would skip the gate as well and the tag would go green with nothing published (hub#895)"
    );

    // Only on a release (ERPlora/app: every push to `main` runs the workflow, tags no longer do).
    // A push whose version is already tagged and a dispatch with `publish: false` publish nothing
    // on purpose; the `version` job is what says which runs are a release.
    assert!(
        block.contains(RELEASE_ONLY),
        "the `{GATE_JOB}` job must be scoped to a release (`{RELEASE_ONLY}`). A push that only \
         touches docs and a manual dispatch with `-f publish=false` are allowed to publish \
         nothing, so demanding publication there would make the gate cry wolf until someone \
         removed it"
    );

    assert!(
        block.contains(GATE_SCRIPT),
        "the `{GATE_JOB}` job no longer calls `{GATE_SCRIPT}`. The decision lives in that script \
         because that is where it can be tested (scripts/release-gate.test.sh); inlining it back \
         into the YAML puts it somewhere that is only ever executed on a tag"
    );
}

#[test]
fn the_gate_sees_every_other_job_of_the_release() {
    let code = workflow_code();
    let ids = job_ids(&code);
    assert!(
        ids.len() >= 5,
        "expected the release workflow to declare the build, android, publish and upload jobs; \
         found {ids:?} — the parser above is probably reading the file wrong"
    );

    let block = gate_block(&code);
    let needs = needs_line(&block);
    assert!(!needs.is_empty(), "the `{GATE_JOB}` job declares no `needs:`");

    // The jobs that run AFTER the gate (they need it) cannot be judged by it: today only the tag
    // job, which exists precisely to act on the gate's verdict.
    let downstream: Vec<&String> = ids
        .iter()
        .filter(|id| {
            *id != GATE_JOB
                && job_block(&code, id).is_some_and(|b| needs_line(&b).contains(GATE_JOB))
        })
        .collect();
    assert_eq!(
        downstream,
        vec![TAG_JOB],
        "only `{TAG_JOB}` may run after `{GATE_JOB}`: anything else downstream of the gate is a \
         channel the gate cannot see"
    );

    let mut missing = Vec::new();
    let mut unreported = Vec::new();
    for id in ids
        .iter()
        .filter(|id| *id != GATE_JOB && !downstream.contains(id))
    {
        if !needs.contains(id.as_str()) {
            missing.push(id.clone());
        }
        // In `needs:` but never handed to the script = in the dependency graph and invisible to
        // the decision, which is worse than absent: it reads as covered.
        if !block.contains(&format!("needs.{id}.result")) {
            unreported.push(id.clone());
        }
    }

    assert!(
        missing.is_empty(),
        "these jobs of `tauri-release.yml` are not in the `needs:` of `{GATE_JOB}`: {missing:?}. \
         The gate is the last job of the release and has to depend on all of them — a publishing \
         job it does not wait for is a channel that can silently ship nothing, which is hub#895 \
         reopened for that channel alone"
    );
    assert!(
        unreported.is_empty(),
        "these jobs are in the `needs:` of `{GATE_JOB}` but their `needs.<job>.result` is never \
         passed to `{GATE_SCRIPT}`: {unreported:?}. The gate would wait for them and then decide \
         without looking at them"
    );
}

/// The tag `v<version>` is what marks a version as released: the next push of the same version
/// releases nothing (scripts/release-version.sh). So it is written LAST, and only when the gate
/// says every channel that had to publish did — a tag on a half-published release would turn
/// "retry the failed job" into "nothing to do".
#[test]
fn the_tag_is_created_only_after_the_gate_passes() {
    let code = workflow_code();
    let block = job_block(&code, TAG_JOB).unwrap_or_else(|| {
        panic!("`tauri-release.yml` has no `{TAG_JOB}:` job: nothing tags a released version")
    });
    assert!(
        needs_line(&block).contains(GATE_JOB),
        "`{TAG_JOB}` must need `{GATE_JOB}`: the tag says «released», and only the gate knows"
    );
    assert!(
        block.contains("needs.release-gate.result == 'success'") && block.contains(RELEASE_ONLY),
        "`{TAG_JOB}` must run only on a release whose gate is green"
    );
    // `always()` is REQUIRED here, not tolerated: the implicit `success()` of a job-level `if:` is
    // also false when an ANCESTOR was skipped, and a store channel declared pending is a skipped
    // ancestor. Without it the tag would never be written while a channel is pending. What keeps
    // a red gate from tagging is the explicit `needs.release-gate.result == 'success'` above.
    assert!(
        block.contains("always()"),
        "`{TAG_JOB}` needs `always()`: a pending (skipped) store job upstream would otherwise \
         skip it and no released version would ever be tagged"
    );
    assert!(
        block.contains("gh release create"),
        "`{TAG_JOB}` must create the tag and the GitHub Release (`gh release create`)"
    );

    // Nothing else tags: a second writer would mark a version released behind the gate's back.
    for id in job_ids(&code).iter().filter(|id| *id != TAG_JOB) {
        let other = job_block(&code, id).unwrap_or_default();
        for writer in ["gh release create", "git tag", "git push"] {
            assert!(
                !other.contains(writer),
                "the job `{id}` runs `{writer}`: only `{TAG_JOB}` may create the tag, after the gate"
            );
        }
    }
}

#[test]
fn the_gate_script_is_committed_and_executable() {
    let path = repo_root().join(GATE_SCRIPT);
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert!(
        contents.starts_with("#!"),
        "{GATE_SCRIPT} has no shebang; the workflow runs it as a program"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("cannot stat the gate script")
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "{GATE_SCRIPT} is not executable ({mode:o}). Git stores that bit, and a release that \
             dies with `Permission denied` in its last job is a release nobody can tell apart \
             from a broken gate"
        );
    }

    // The tests of the decision itself. They are shipped next to the script so that whoever
    // changes the rules has somewhere to change them; asserting the file exists keeps the pair
    // from drifting into a script with no tests.
    assert!(
        repo_root().join("scripts/release-gate.test.sh").is_file(),
        "scripts/release-gate.test.sh is gone: the gate's rules would have no test at all, and \
         the only other place they run is a tag"
    );
}

/// A tag that "published" but left the change waiting for a click published nothing (2026-08-19).
///
/// `changesNotSentForReview: true` uploads the AAB and stops: the release sits in Play Console's
/// *Changes not yet submitted for review* until a human presses the button. That is not a
/// theoretical risk — it is how the very first listing spent five days. On 2026-08-19 the console
/// held **14 changes** saved since the 14th, `Submission activity` was empty, and the app was
/// still `Draft` under the temporary name `com.erplora.app (unreviewed)`. Nobody had rejected
/// anything; nobody had sent anything either.
///
/// A channel whose last step needs a human is not automated, and worse, it looks automated: the
/// job goes green, the gate goes green, and the release is parked. So the flag has to stay
/// `false` — and if someone ever needs the old behaviour back, this test is where they have to
/// come and say why.
#[test]
fn the_play_upload_actually_sends_the_release_for_review() {
    let code = workflow_code();
    assert!(
        code.contains("changesNotSentForReview: false"),
        "the Google Play step must send the release for review. With \
         `changesNotSentForReview: true` the AAB lands in the console as a pending change and \
         waits for someone to notice — which is exactly how 14 changes sat unsubmitted for five \
         days while every tag reported success"
    );
}
