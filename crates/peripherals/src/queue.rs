//! Print queue with retries, so a job is not lost when the printer is switched off or out of paper
//! and does not answer.

use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::{oneshot, Mutex};

use crate::discovery::NetworkTarget;
use crate::{PeripheralError, Result};

#[derive(Debug, Clone)]
pub struct PrintJob {
    pub job_id: Option<String>,
    pub target: NetworkTarget,
    /// Bytes already rendered by `escpos::render_document`.
    pub payload: Vec<u8>,
    /// Attempts made so far.
    pub attempts: u32,
}

/// What happened to a job; the consumer reports it as `print_complete` / `print_error`.
#[derive(Debug, Clone)]
pub enum JobOutcome {
    Completed { job_id: Option<String> },
    Failed { job_id: Option<String>, error: String },
}

/// Retry policy plus the per-attempt deadlines that keep the worker from ever blocking on the
/// OS network timeouts.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff_ms: u64,
    /// Deadline for the TCP `connect` of a single attempt. Without it, a printer that absorbs
    /// the SYN — loose cable, IP that is no longer hers, filtered port — parks the worker for
    /// the OS connect timeout (tens of seconds) and stalls every other printer's jobs. A LAN
    /// printer that is up answers in milliseconds.
    pub connect_timeout_ms: u64,
    /// Deadline for writing the payload once connected. A printer that accepts the connection
    /// and then wedges (never reads) would otherwise hold the worker until TCP retransmission
    /// gives up, which takes minutes.
    pub write_timeout_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_attempts: 3, backoff_ms: 2000, connect_timeout_ms: 3000, write_timeout_ms: 10_000 }
    }
}

/// A job in the worker's channel, with where to answer its outcome when someone waits for it.
type Queued = (PrintJob, Option<oneshot::Sender<JobOutcome>>);

/// Queues jobs and sends them to the printer's TCP socket, retrying by the policy.
pub struct PrintQueue {
    policy: RetryPolicy,
    /// Producer side: `enqueue` and `print` push jobs here.
    tx: UnboundedSender<Queued>,
    /// Consumer side, behind a `Mutex` so that `run(&self)` can take `&mut` to receive.
    rx: Mutex<UnboundedReceiver<Queued>>,
}

impl PrintQueue {
    pub fn new(policy: RetryPolicy) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { policy, tx, rx: Mutex::new(rx) }
    }

    /// Queues a job and answers on the hand-over, before anything is printed.
    pub fn enqueue(&self, job: PrintJob) -> Result<()> {
        self.push(job, None)
    }

    /// Prints a job and answers with what happened to the PAPER: `Ok` once the bytes
    /// are on the printer, the unreachable error once every retry failed. The job still goes
    /// through the single worker, so it never interleaves with another job's bytes, and its
    /// outcome still reaches the `run` stream.
    ///
    /// `enqueue` answers on the hand-over: with the network printer switched off it says «done»
    /// for a ticket that never comes out, so the till is not told and the device draining the
    /// hub's queue would confirm it printed.
    pub async fn print(&self, job: PrintJob) -> Result<()> {
        let (reply, outcome) = oneshot::channel();
        self.push(job, Some(reply))?;
        match outcome.await {
            Ok(JobOutcome::Completed { .. }) => Ok(()),
            Ok(JobOutcome::Failed { error, .. }) => Err(PeripheralError::Unreachable(error)),
            // The worker dropped the job without an outcome: it stopped, nothing was printed.
            Err(_) => Err(PeripheralError::Unreachable(
                "the print queue stopped before printing the job".into(),
            )),
        }
    }

    fn push(&self, job: PrintJob, reply: Option<oneshot::Sender<JobOutcome>>) -> Result<()> {
        self.tx.send((job, reply)).map_err(|e| {
            PeripheralError::InvalidPayload(format!("cola de impresión cerrada: {e}"))
        })
    }

    /// Sends one job: opens the socket to the target and writes the payload. One attempt, with
    /// its own deadlines — this call must never hang on the OS network timeouts,
    /// because the worker processes jobs serially and every other printer waits behind it. A
    /// timed-out attempt returns `Err`, so `process` retries it under the same `job_id` instead of
    /// duplicating the print.
    pub async fn send_once(&self, job: &PrintJob) -> Result<()> {
        let addr = job.target.socket_addr();
        let mut stream = match tokio::time::timeout(
            Duration::from_millis(self.policy.connect_timeout_ms),
            TcpStream::connect(&addr),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => return Err(e.into()),
            // The connect was absorbed (loose cable, stale IP, filtered port): unreachable.
            Err(_elapsed) => {
                return Err(PeripheralError::Unreachable(format!(
                    "{addr}: connect timed out after {} ms",
                    self.policy.connect_timeout_ms
                )))
            }
        };
        match tokio::time::timeout(
            Duration::from_millis(self.policy.write_timeout_ms),
            stream.write_all(&job.payload),
        )
        .await
        {
            Ok(result) => result.map_err(Into::into),
            // Accepted the connection and then wedged without reading: unreachable too.
            Err(_elapsed) => Err(PeripheralError::Unreachable(format!(
                "{addr}: write timed out after {} ms",
                self.policy.write_timeout_ms
            ))),
        }
    }

    /// Worker loop: drains the queue, retrying by the policy, and sends every `JobOutcome` to
    /// `outcomes` (the consumer maps it to `print_complete` / `print_error`).
    pub async fn run(&self, outcomes: UnboundedSender<JobOutcome>) {
        while let Some((job, reply)) = {
            let mut rx = self.rx.lock().await;
            rx.recv().await
        } {
            let outcome = self.process(job).await;
            match &outcome {
                JobOutcome::Completed { job_id } => {
                    tracing::info!(?job_id, "print job completed");
                }
                JobOutcome::Failed { job_id, error } => {
                    tracing::warn!(?job_id, %error, "print job failed");
                }
            }
            // Whoever waits on `print` hears it first; one that gave up only loses its answer.
            if let Some(reply) = reply {
                let _ = reply.send(outcome.clone());
            }
            // If the consumer is gone, only the report is lost, not the processing of the queue.
            let _ = outcomes.send(outcome);
        }
    }

    async fn process(&self, mut job: PrintJob) -> JobOutcome {
        loop {
            match self.send_once(&job).await {
                Ok(()) => return JobOutcome::Completed { job_id: job.job_id },
                Err(e) => {
                    job.attempts += 1;
                    if job.attempts < self.policy.max_attempts {
                        tracing::debug!(
                            attempts = job.attempts,
                            max = self.policy.max_attempts,
                            error = %e,
                            "retrying print job",
                        );
                        tokio::time::sleep(Duration::from_millis(self.policy.backoff_ms)).await;
                    } else {
                        return JobOutcome::Failed { job_id: job.job_id, error: e.to_string() };
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escpos::{self, DocumentType};
    use crate::test_support::{unreachable_target, MockPrinter};
    use std::sync::Arc;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    /// Fast retries: tests must not pay the 2 s production backoff. Deadlines stay generous so
    /// only the black-hole tests, which set their own tight values, exercise them.
    fn fast_policy(max_attempts: u32) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            backoff_ms: 10,
            connect_timeout_ms: 1000,
            write_timeout_ms: 1000,
        }
    }

    /// A "black hole" printer: a local address whose SYNs are absorbed — neither accepted nor
    /// refused — so a bare `connect` against it hangs until some deadline fires. Models a printer
    /// that absorbs the connect (loose cable, stale IP, filtered port) without hardware. Keep the
    /// value alive: dropping it frees the port again.
    ///
    /// No single loopback trick absorbs on every kernel: macOS drops the SYN to a
    /// bound socket that does not listen but resets a connect to a saturated backlog; Linux
    /// does the opposite. So `start` builds the first shape and keeps it only if a probe
    /// connect really hangs, and falls back to the second one otherwise.
    struct BlackHolePrinter {
        target: NetworkTarget,
        _hole: BlackHole,
    }

    enum BlackHole {
        /// Bound but not listening: macOS drops the SYN.
        Unlistened { _socket: tokio::net::TcpSocket },
        /// A listener whose backlog is full and never accepts: Linux drops further SYNs.
        SaturatedBacklog { _listener: TcpListener, _guards: Vec<TcpStream> },
    }

    impl BlackHolePrinter {
        const PROBE: Duration = Duration::from_millis(150);

        async fn start() -> Self {
            let socket = tokio::net::TcpSocket::new_v4().unwrap();
            socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
            let addr = socket.local_addr().unwrap();
            if Self::absorbs(addr).await {
                return Self::at(addr, BlackHole::Unlistened { _socket: socket });
            }

            let listener = socket.listen(1).unwrap();
            let mut guards = Vec::new();
            for _ in 0..4 {
                match tokio::time::timeout(Self::PROBE, TcpStream::connect(addr)).await {
                    Ok(Ok(stream)) => guards.push(stream),
                    // The backlog is full: this connect already hangs, the hole is ready.
                    _ => break,
                }
            }
            Self::at(addr, BlackHole::SaturatedBacklog { _listener: listener, _guards: guards })
        }

        async fn absorbs(addr: std::net::SocketAddr) -> bool {
            tokio::time::timeout(Self::PROBE, TcpStream::connect(addr)).await.is_err()
        }

        fn at(addr: std::net::SocketAddr, hole: BlackHole) -> Self {
            Self {
                target: NetworkTarget {
                    host: addr.ip().to_string(),
                    port: addr.port(),
                },
                _hole: hole,
            }
        }
    }

    /// A printer that is switched off but keeps its address. The socket is bound, so no other
    /// socket can take the port, yet it does not listen, so no connect gets through (Linux
    /// refuses it, macOS drops the SYN) — what a powered-off printer on the LAN does. `switch_on`
    /// listens on that same socket, so the port never goes back to the OS in between.
    struct SwitchedOffPrinter {
        socket: tokio::net::TcpSocket,
        addr: std::net::SocketAddr,
        target: NetworkTarget,
    }

    impl SwitchedOffPrinter {
        fn new() -> Self {
            let socket = tokio::net::TcpSocket::new_v4().unwrap();
            socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
            let addr = socket.local_addr().unwrap();
            Self {
                socket,
                addr,
                target: NetworkTarget {
                    host: addr.ip().to_string(),
                    port: addr.port(),
                },
            }
        }

        fn switch_on(self) -> TcpListener {
            self.socket.listen(16).expect("listen on the reserved port")
        }
    }

    /// Starts the queue's worker and returns the receiver of its outcomes.
    fn spawn_worker(queue: Arc<PrintQueue>) -> UnboundedReceiver<JobOutcome> {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move { queue.run(tx).await });
        rx
    }

    fn job(target: NetworkTarget, payload: &[u8]) -> PrintJob {
        PrintJob {
            job_id: Some("j1".into()),
            target,
            payload: payload.to_vec(),
            attempts: 0,
        }
    }

    /// **What was queued reaches the printer byte for byte.** That is the whole point of the
    /// queue: if the worker mangles or loses the payload, nobody notices until they look at the
    /// paper.
    #[tokio::test]
    async fn an_enqueued_job_reaches_the_printer_byte_for_byte() {
        let mock = MockPrinter::start().await;
        let queue = Arc::new(PrintQueue::new(fast_policy(3)));
        let mut outcomes = spawn_worker(queue.clone());

        let payload = b"hello escpos";
        queue.enqueue(job(mock.target.clone(), payload)).unwrap();

        assert!(
            matches!(outcomes.recv().await, Some(JobOutcome::Completed { .. })),
            "a send that arrives is reported as completed"
        );
        assert_eq!(mock.captured_bytes().await, payload);
        assert!(mock.accepted(), "the connection to the printer was opened");
    }

    /// A real receipt rendered by `escpos` survives the whole trip: the `GS V 1` cut and the
    /// business name (cp437) have to leave through the wire, not just the renderer.
    #[tokio::test]
    async fn a_rendered_receipt_travels_whole_including_the_paper_cut() {
        let mock = MockPrinter::start().await;
        let queue = Arc::new(PrintQueue::new(fast_policy(3)));
        let mut outcomes = spawn_worker(queue.clone());

        let bytes = escpos::render_document(
            DocumentType::Receipt,
            &serde_json::json!({
                "business_name": "Bar Pepe",
                "receipt_id": "T-001",
                "items": [{ "name": "Cafe", "quantity": 2, "total": 3.0 }],
                "total": 3.0,
            }),
        )
        .expect("a well-formed receipt renders");
        queue.enqueue(job(mock.target.clone(), &bytes)).unwrap();
        outcomes.recv().await.expect("there is an outcome");

        let got = mock.captured_bytes().await;
        assert!(
            got.windows(3).any(|w| w == [0x1d, 0x56, 0x01]),
            "without the cut, the next ticket comes out stuck to this one"
        );
        assert!(
            got.windows(8).any(|w| w == b"Bar Pepe"),
            "the business name must reach the paper"
        );
    }

    /// The switched-off printer is a real off printer AND keeps its address: while off, a connect
    /// does not get through and no other socket can take the port; once switched on, the very same
    /// address accepts. Rebinding a dropped port instead would let a parallel test grab it in
    /// between and fail with `AddrInUse`.
    #[tokio::test]
    async fn a_switched_off_printer_refuses_connections_and_keeps_its_port() {
        let printer = SwitchedOffPrinter::new();
        let addr = printer.addr;

        // Linux refuses the connect outright; macOS drops the SYN. Either way it never connects.
        let off = tokio::time::timeout(Duration::from_millis(200), TcpStream::connect(addr)).await;
        assert!(
            !matches!(off, Ok(Ok(_))),
            "an off printer must not take the connection, got {off:?}"
        );
        let stolen = TcpListener::bind(addr)
            .await
            .expect_err("the port stays reserved");
        assert_eq!(stolen.kind(), std::io::ErrorKind::AddrInUse);

        let listener = printer.switch_on();
        let (connected, accepted) = tokio::join!(TcpStream::connect(addr), listener.accept());
        connected.expect("a switched-on printer accepts on the same address");
        accepted.expect("and the listener sees the connection");
    }

    /// A switched-on printer serves EVERY connection, as a real one does, and hands back the bytes
    /// of the first one that brought any. One that arrives empty — the late handshake of an
    /// attempt that already gave up — is neither the job nor a reason to switch off again: a
    /// printer that served ONE connection and then closed its port would turn it into a lost job
    /// (app#3). The listener comes back still switched on, so the test can check what else
    /// reached the paper afterwards.
    async fn serve_until_a_job(listener: TcpListener) -> (Vec<u8>, TcpListener) {
        loop {
            let (mut sock, _) = listener.accept().await.expect("the printer accepts");
            let mut buf = Vec::new();
            let _ = sock.read_to_end(&mut buf).await;
            if !buf.is_empty() {
                return (buf, listener);
            }
        }
    }

    /// What the switched-on printer receives AFTER the job is on paper, within `wait`: a second
    /// copy of the job is a ticket printed twice, which a printer that accepts once and switches
    /// off could not tell from one. Empty handshakes (attempts that gave up) are not paper.
    async fn printed_afterwards(printer: &TcpListener, wait: Duration) -> Option<Vec<u8>> {
        tokio::time::timeout(wait, async {
            loop {
                let (mut sock, _) = printer.accept().await.expect("the printer accepts");
                let mut buf = Vec::new();
                let _ = sock.read_to_end(&mut buf).await;
                if !buf.is_empty() {
                    return buf;
                }
            }
        })
        .await
        .ok()
    }

    /// **A printer switched off for a moment does not lose the job.** This is what the queue is
    /// for: the first attempt fails with the port closed and a later retry, with the printer back,
    /// delivers. Without it a ticket is lost every time someone trips over the cable.
    #[tokio::test]
    async fn a_printer_that_comes_back_still_gets_its_job() {
        // Off for longer than one attempt's connect deadline, so the first attempt FAILS while the
        // printer is off on every kernel: Linux refuses the connect; macOS drops the SYN and the
        // deadline gives up. With a deadline longer than the time off, macOS keeps that first SYN
        // pending and delivers it on a retransmission: the job prints without a single retry, and
        // a queue that never retries would pass this test there (app#3).
        const CONNECT_DEADLINE_MS: u64 = 100;
        const OFF_FOR: Duration = Duration::from_millis(300);
        let printer = SwitchedOffPrinter::new();
        let (addr, target) = (printer.addr, printer.target.clone());
        let revive = tokio::spawn(async move {
            tokio::time::sleep(OFF_FOR).await;
            let listener = printer.switch_on();
            // An attempt that gave up just as the printer came back can still finish its
            // handshake: its connection reaches the printer EMPTY, and first — on macOS a connect
            // deadline can race a SYN retransmission that way (app#3). Knock that way on purpose,
            // so every run meets it.
            let knock = TcpStream::connect(addr).await;
            drop(knock.expect("the switched-on printer answers"));
            serve_until_a_job(listener).await
        });

        // Deterministic by ORDER, not by speed: the worker and the printer share this test's one
        // thread (`#[tokio::test]` is current-thread) and its timer, which fires deadlines in
        // order. The printer's 300 ms falls after the first attempt's 100 ms deadline and before
        // the 49 back-offs of 20 ms (980 ms) the queue waits at least before giving up, however
        // late a loaded runner wakes the thread.
        let queue = Arc::new(PrintQueue::new(RetryPolicy {
            max_attempts: 50,
            backoff_ms: 20,
            connect_timeout_ms: CONNECT_DEADLINE_MS,
            write_timeout_ms: 1000,
        }));
        let mut outcomes = spawn_worker(queue.clone());
        queue.enqueue(job(target, b"resilient job")).unwrap();

        assert!(
            matches!(outcomes.recv().await, Some(JobOutcome::Completed { .. })),
            "the job completes after the recovery, it is not dropped"
        );
        let (paper, printer) = revive.await.unwrap();
        assert_eq!(
            paper,
            b"resilient job".to_vec(),
            "and the delivered payload is the original, not one truncated by the failed attempts"
        );
        // And ONCE: a queue that went on sending after the success would hand the printer a
        // second copy — it is already in the printer's backlog by the time `Completed` arrives.
        assert_eq!(
            printed_afterwards(&printer, Duration::from_millis(50)).await,
            None,
            "and nothing else reaches the paper: the job prints only once"
        );
    }

    /// **When the attempts run out the job is reported as failed, it does not disappear.** The
    /// consumer maps this outcome to `print_error`; an optimistic `Completed` here would be a
    /// ticket nobody printed and nobody missed.
    #[tokio::test]
    async fn a_job_that_never_lands_ends_as_failed_with_the_reason() {
        let queue = Arc::new(PrintQueue::new(fast_policy(3)));
        let mut outcomes = spawn_worker(queue.clone());

        queue
            .enqueue(job(unreachable_target(), b"never lands"))
            .unwrap();

        match outcomes.recv().await {
            Some(JobOutcome::Failed { job_id, error }) => {
                assert_eq!(job_id.as_deref(), Some("j1"), "the id comes back so it can be correlated");
                assert!(!error.is_empty(), "the failure has to say why");
            }
            other => panic!("an unreachable printer cannot complete: {other:?}"),
        }
    }

    /// The policy is **honoured**: with `max_attempts: 1` there is no retry. Pinning it stops a
    /// miscounted loop from turning one attempt into three (or the default of 3 into one, which
    /// would remove the recovery tested above).
    ///
    /// Measured against the backoff, not against a fixed 10 ms: a retry is only visible as the
    /// backoff it waits. On Windows a refused loopback connect is not instant (the stack retries
    /// the SYN), so the single attempt alone takes about a second there. A long backoff keeps the
    /// test about the retry, not about how fast each OS says «refused».
    #[tokio::test]
    async fn a_single_attempt_policy_does_not_retry() {
        let policy = RetryPolicy {
            backoff_ms: 10_000,
            ..fast_policy(1)
        };
        let backoff = Duration::from_millis(policy.backoff_ms);
        let queue = Arc::new(PrintQueue::new(policy));
        let started = std::time::Instant::now();
        let outcome = queue
            .process(job(unreachable_target(), b"one shot"))
            .await;

        assert!(matches!(outcome, JobOutcome::Failed { .. }));
        assert!(
            started.elapsed() < backoff,
            "a single attempt must not wait any backoff (took {:?}, backoff {backoff:?})",
            started.elapsed()
        );
    }

    /// Queueing into a queue whose worker died is an explicit error, not a job that evaporates.
    #[tokio::test]
    async fn enqueueing_into_a_closed_queue_is_refused() {
        let queue = PrintQueue::new(fast_policy(1));
        queue.rx.lock().await.close();
        let err = queue
            .enqueue(job(unreachable_target(), b"nowhere"))
            .expect_err("a closed queue takes no jobs");
        assert!(matches!(err, PeripheralError::InvalidPayload(_)));
    }

    /// The black hole really absorbs the connect on this OS: neither accepted nor
    /// refused. If it resets instead, the deadline tests below go green or red for the wrong
    /// reason — a fast `ConnectionReset` never exercises `connect_timeout_ms`.
    #[tokio::test]
    async fn the_black_hole_printer_absorbs_the_connect() {
        let hole = BlackHolePrinter::start().await;
        let addr = hole.target.socket_addr();
        let probe =
            tokio::time::timeout(Duration::from_millis(300), TcpStream::connect(&addr)).await;
        assert!(probe.is_err(), "a black hole must leave the connect hanging, got {probe:?}");
    }

    /// **The connect deadline is the queue's own, not the OS one.** Against a printer
    /// that absorbs the connection without accepting or refusing it, `send_once` must give up
    /// within its configured deadline; inheriting the OS timeout means tens of seconds with the
    /// worker parked and every other printer's jobs behind it.
    #[tokio::test]
    async fn send_once_gives_up_within_its_own_connect_deadline() {
        let hole = BlackHolePrinter::start().await;
        let queue = PrintQueue::new(RetryPolicy {
            max_attempts: 1,
            backoff_ms: 10,
            connect_timeout_ms: 100,
            write_timeout_ms: 1000,
        });

        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            queue.send_once(&job(hole.target.clone(), b"stuck")),
        )
        .await
        .expect("send_once must give up within its own connect deadline, not the OS one");

        assert!(
            matches!(result, Err(PeripheralError::Unreachable(_))),
            "an absorbed connect is the printer being unreachable, got {result:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the 100 ms deadline must bound the attempt (took {:?})",
            started.elapsed()
        );
    }

    /// **A printer that accepts and then wedges cannot hold the worker either.** The write gets
    /// its own deadline: a payload bigger than the socket buffers against a peer that never
    /// reads would otherwise block `write_all` until TCP retransmission gives up (minutes).
    ///
    /// Not on Windows: there the premise does not hold. Winsock takes the whole 16 MiB of a
    /// non-blocking send into its own buffers while the peer reads nothing, so `write_all` returns
    /// `Ok` and no stall exists for the deadline to cut. The deadline is the same code on every OS
    /// (`tokio::time::timeout` around the write) and Linux and macOS still exercise it.
    #[tokio::test]
    #[cfg_attr(
        windows,
        ignore = "Winsock buffers the whole payload of a non-blocking send: the write never stalls"
    )]
    async fn send_once_gives_up_on_a_printer_that_accepts_but_never_reads() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let wedged = tokio::spawn(async move {
            let (_sock, _) = listener.accept().await.unwrap();
            // Hold the socket open without reading a byte: the sender's buffers fill and stay full.
            tokio::time::sleep(Duration::from_secs(10)).await;
        });

        let queue = PrintQueue::new(RetryPolicy {
            max_attempts: 1,
            backoff_ms: 10,
            connect_timeout_ms: 1000,
            write_timeout_ms: 100,
        });
        // Well past what loopback socket buffers absorb, so the write really stalls.
        let payload = vec![0x20u8; 16 * 1024 * 1024];

        let result = tokio::time::timeout(
            Duration::from_secs(2),
            queue.send_once(&job(
                NetworkTarget {
                    host: addr.ip().to_string(),
                    port: addr.port(),
                },
                &payload,
            )),
        )
        .await
        .expect("the write must give up within its own deadline, not the OS retransmission one");

        assert!(
            matches!(result, Err(PeripheralError::Unreachable(_))),
            "a wedged printer is unreachable for the queue, got {result:?}"
        );
        wedged.abort();
    }

    /// **One dead printer does not stop the drain for the healthy ones** (bar, kitchen and till
    /// share the one serial worker). The job against the black hole fails within its
    /// bounded retries and the next job — a different, healthy printer — still prints.
    #[tokio::test]
    async fn one_dead_printer_does_not_stall_the_healthy_ones_jobs() {
        let hole = BlackHolePrinter::start().await;
        let mock = MockPrinter::start().await;
        let queue = Arc::new(PrintQueue::new(RetryPolicy {
            max_attempts: 3,
            backoff_ms: 10,
            connect_timeout_ms: 100,
            write_timeout_ms: 1000,
        }));
        let mut outcomes = spawn_worker(queue.clone());

        let started = std::time::Instant::now();
        queue
            .enqueue(PrintJob {
                job_id: Some("dead".into()),
                target: hole.target.clone(),
                payload: b"never lands".to_vec(),
                attempts: 0,
            })
            .unwrap();
        queue
            .enqueue(PrintJob {
                job_id: Some("alive".into()),
                target: mock.target.clone(),
                payload: b"bar ticket".to_vec(),
                attempts: 0,
            })
            .unwrap();

        let first = tokio::time::timeout(Duration::from_secs(2), outcomes.recv())
            .await
            .expect("the dead printer's job must fail within its bounded retries, not the OS timeout")
            .unwrap();
        match first {
            JobOutcome::Failed { job_id, .. } => {
                assert_eq!(job_id.as_deref(), Some("dead"), "the failed job keeps its id for retry")
            }
            other => panic!("a black-holed printer cannot complete: {other:?}"),
        }
        // The failure must have waited out the connect deadline on every attempt: an instant
        // refusal or reset would also end as `Failed`, without exercising it.
        assert!(
            started.elapsed() >= Duration::from_millis(300),
            "three attempts must each wait out the 100 ms connect deadline (took {:?})",
            started.elapsed()
        );

        let second = tokio::time::timeout(Duration::from_secs(2), outcomes.recv())
            .await
            .expect("the healthy printer's job must follow right behind the bounded failure")
            .unwrap();
        match second {
            JobOutcome::Completed { job_id } => assert_eq!(job_id.as_deref(), Some("alive")),
            other => panic!("the healthy printer must still get its job: {other:?}"),
        }
        assert_eq!(mock.captured_bytes().await, b"bar ticket");
    }

    // ── Whoever prints hears what happened to the PAPER, not to the hand-over ───────────────────
    // `enqueue` answers as soon as the job is in memory: with the network printer switched off the
    // till would hear «done» and the device draining the hub's queue would confirm «printed» for a
    // ticket that never came out. `print` waits for the job's own outcome, retries included.

    /// A switched-off printer is an ERROR for whoever asked, with the stable unreachable code the
    /// shell already maps to «not printed».
    #[tokio::test]
    async fn printing_to_a_switched_off_printer_fails_for_the_caller() {
        let printer = SwitchedOffPrinter::new();
        let queue = Arc::new(PrintQueue::new(fast_policy(2)));
        let _outcomes = spawn_worker(queue.clone());

        let err = queue
            .print(job(printer.target.clone(), b"ticket"))
            .await
            .expect_err("a ticket that never reached the printer cannot be reported as printed");

        assert_eq!(err.code(), "printer_unreachable", "got {err}");
    }

    /// `print` answers only once the bytes are on the printer, never on the hand-over.
    #[tokio::test]
    async fn printing_answers_after_the_paper_reached_the_printer() {
        let mock = MockPrinter::start().await;
        let queue = Arc::new(PrintQueue::new(fast_policy(3)));
        let _outcomes = spawn_worker(queue.clone());

        queue
            .print(job(mock.target.clone(), b"kitchen order"))
            .await
            .expect("a printer that takes the bytes is a printed job");

        assert_eq!(mock.captured_bytes().await, b"kitchen order");
    }

    /// The retries still happen before the answer: a printer that comes back on during them prints,
    /// and the caller hears «printed», not the first failed attempt.
    #[tokio::test]
    async fn a_printer_switched_on_during_the_retries_is_a_printed_job() {
        let printer = SwitchedOffPrinter::new();
        let target = printer.target.clone();
        let policy = RetryPolicy { max_attempts: 20, backoff_ms: 50, connect_timeout_ms: 100, write_timeout_ms: 1000 };
        let queue = Arc::new(PrintQueue::new(policy));
        let _outcomes = spawn_worker(queue.clone());

        let printing = tokio::spawn({
            let queue = queue.clone();
            async move { queue.print(job(target, b"late")).await }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        let (got, _printer) = serve_until_a_job(printer.switch_on()).await;

        printing.await.unwrap().expect("printed on a later attempt");
        assert_eq!(got, b"late");
    }

    /// The other producers of the worker (the outcome stream the app logs) keep hearing every job.
    #[tokio::test]
    async fn a_printed_job_is_still_reported_on_the_outcome_stream() {
        let mock = MockPrinter::start().await;
        let queue = Arc::new(PrintQueue::new(fast_policy(3)));
        let mut outcomes = spawn_worker(queue.clone());

        queue.print(job(mock.target.clone(), b"x")).await.unwrap();

        assert!(matches!(outcomes.recv().await, Some(JobOutcome::Completed { .. })));
    }

    /// A job the worker dropped without an outcome (it stopped mid-job) was NOT printed: the caller
    /// hears the unreachable error, never «printed». It is the only test that catches the dropped
    /// reply mapped to `Ok(())`.
    #[tokio::test]
    async fn a_job_the_worker_dropped_without_an_outcome_is_not_printed() {
        let queue = Arc::new(PrintQueue::new(fast_policy(1)));
        // No worker running: the job is pulled by hand and its reply dropped unanswered, which is
        // what a worker that dies between taking the job and reporting it leaves behind.
        let printing = tokio::spawn({
            let queue = queue.clone();
            async move { queue.print(job(unreachable_target(), b"x")).await }
        });
        let (_job, reply) = queue.rx.lock().await.recv().await.expect("the job was handed over");
        drop(reply);

        let err = printing.await.unwrap().expect_err("a job nobody printed cannot be «printed»");
        assert_eq!(err.code(), "printer_unreachable", "got {err}");
    }
}
