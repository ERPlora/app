//! A CUPS for the tests: it records every call and answers whatever the test scripted, so the
//! transport is exercised without printing on anyone's desk. Shared by the tests of [`super`]
//! and of [`super::discovery`].

use std::cell::RefCell;

use super::cups::{CupsClient, CupsOutput};
use crate::Result;

/// `(program, args, stdin)` of every call the fake was asked to make.
type RecordedCalls = Vec<(String, Vec<String>, Vec<u8>)>;

/// `(args, answers)` for an argv asked more than once — see [`FakeCups::sequenced`].
type ScriptedSequence = Vec<(Vec<&'static str>, Vec<CupsOutput>)>;

/// Records what would have been run, and answers whatever the test wants it to — per argv,
/// so a discovery that asks two questions (`-e`, then `-v`) can be given two answers.
pub(super) struct FakeCups {
    /// `(args, answer)`; an empty `args` answers any call.
    pub(super) answers: Vec<(Vec<&'static str>, CupsOutput)>,
    /// `(args, answers)` for an argv that is asked MORE THAN ONCE and has to answer
    /// differently each time — the shape the completion wait needs, where the queue listing
    /// must change between polls for the wait to be provably a wait and not a single look.
    /// Consumed in order, the last one keeps answering, and it is checked before
    /// [`FakeCups::answers`].
    pub(super) sequenced: RefCell<ScriptedSequence>,
    pub(super) calls: RefCell<RecordedCalls>,
}

impl Default for FakeCups {
    fn default() -> Self {
        Self {
            answers: Vec::new(),
            sequenced: RefCell::new(Vec::new()),
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl FakeCups {
    pub(super) fn answering(answer: CupsOutput) -> Self {
        Self { answers: vec![(Vec::new(), answer)], ..Default::default() }
    }
    pub(super) fn ok() -> Self {
        Self::answering(CupsOutput {
            success: true,
            stdout: "request id is Star_TSP143-7 (1 file(s))".into(),
            stderr: String::new(),
        })
    }
    /// A CUPS whose `lpoptions -p <queue>` answers `state` and whose `lp` accepts the job.
    /// The pre-flight asks the first question before it dares the second.
    pub(super) fn queue_state(queue: &'static str, state: &str) -> Self {
        let ok = |stdout: &str| CupsOutput {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
        };
        Self {
            answers: vec![
                (vec!["-p", queue], ok(state)),
                // And the job leaves the queue, which is what "it printed" means.
                (vec!["-W", "not-completed", "-o", queue], ok("")),
                (Vec::new(), ok("request id is Star_TSP143-7 (1 file(s))")),
            ],
            ..Default::default()
        }
    }

    /// The whole conversation for one submit and its wait: `lpoptions -p <queue>` answers
    /// `states` in order (idle at the pre-flight, then whatever the backend reports once it
    /// has *tried* to print), `lp` accepts the job as `<queue>-7`, and
    /// `lpstat -W not-completed -o <queue>` answers `listings` in order, one per poll. The
    /// last of each keeps answering, so a listing that never clears is a job that never left.
    pub(super) fn spooling(
        queue: &'static str,
        states: Vec<&'static str>,
        listings: Vec<&'static str>,
    ) -> Self {
        let ok = |stdout: &str| CupsOutput {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
        };
        let sequence =
            |outs: Vec<&'static str>| outs.into_iter().map(ok).collect::<Vec<_>>();
        Self {
            answers: vec![(
                Vec::new(),
                ok(&format!("request id is {queue}-7 (1 file(s))")),
            )],
            sequenced: RefCell::new(vec![
                (vec!["-p", queue], sequence(states)),
                (vec!["-W", "not-completed", "-o", queue], sequence(listings)),
            ]),
            ..Default::default()
        }
    }

    /// A healthy CUPS whose `lpstat -e` prints `destinations` and whose `lpstat -v` prints
    /// `devices`.
    pub(super) fn listing(destinations: &str, devices: &str) -> Self {
        let ok = |stdout: &str| CupsOutput {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
        };
        Self {
            answers: vec![(vec!["-e"], ok(destinations)), (vec!["-v"], ok(devices))],
            ..Default::default()
        }
    }
}

impl CupsClient for FakeCups {
    fn run(&self, program: &str, args: &[&str], stdin: &[u8]) -> Result<CupsOutput> {
        self.calls.borrow_mut().push((
            program.to_string(),
            args.iter().map(|a| a.to_string()).collect(),
            stdin.to_vec(),
        ));
        if let Some((_, queued)) = self
            .sequenced
            .borrow_mut()
            .iter_mut()
            .find(|(for_args, _)| for_args.as_slice() == args)
        {
            let answer = queued.first().cloned().expect("a sequence answers at least once");
            if queued.len() > 1 {
                queued.remove(0);
            }
            return Ok(answer);
        }
        let scripted = self
            .answers
            .iter()
            .find(|(for_args, _)| for_args.is_empty() || for_args.as_slice() == args)
            .map(|(_, answer)| answer.clone());
        Ok(scripted.unwrap_or_else(|| CupsOutput {
            success: false,
            stdout: String::new(),
            stderr: format!("fake cups: nothing scripted for `{program} {args:?}`"),
        }))
    }
}

/// A successful CUPS answer carrying `stdout`.
pub(super) fn cups_said(stdout: &str) -> CupsOutput {
    CupsOutput { success: true, stdout: stdout.into(), stderr: String::new() }
}

/// A CUPS answer for a queue that is idle and taking work.
pub(super) const LPOPTIONS_IDLE: &str =
    "printer-is-accepting-jobs=true printer-state=3 printer-state-reasons=none";
