//! Tokio facade backed by one dedicated standard worker thread per session.
//!
//! Every handle serializes commands through a FIFO channel. The blocking
//! typestate always remains on its worker, including when an awaiting future is
//! dropped. Progress uses a Tokio watch channel and therefore coalesces samples
//! when a receiver is slower than Windows.
//!
//! Every public future returned by this module is [`Send`].

use std::borrow::Borrow;
use std::ffi::OsStr;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::mpsc;
use std::task::{Context, Poll};

use ::tokio::sync::{oneshot, watch};

use crate::{
    AffectedApplications, CancellationHandle, Error, ErrorKind, Filter, FilterAction, FilterTarget,
    OperationNotStarted, OperationOutcome, ProcessIdentity, Progress, RecoveryOutcome,
    ResourceBatch, Result, SessionKey, ShutdownOptions,
};

type Job = Box<dyn FnOnce(&mut WorkerState) + Send + 'static>;

enum WorkerState {
    Primary(crate::RestartSession),
    Joined(crate::JoinedSession),
    Pending(crate::RestartPending),
    Completion(crate::RecoveryCompletion),
    Empty,
}

struct Worker {
    sender: mpsc::Sender<Job>,
}

impl Worker {
    fn send(&self, job: Job) -> Result<()> {
        self.sender.send(job).map_err(|_| worker_unavailable())
    }

    async fn call<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut WorkerState) -> T + Send + 'static,
    {
        let (sender, receiver) = oneshot::channel();
        self.send(Box::new(move |state| {
            let _ = sender.send(operation(state));
        }))?;
        receiver.await.map_err(|_| worker_unavailable())
    }
}

struct PrimaryInit {
    key: SessionKey,
    cancellation: CancellationHandle,
}

async fn start_primary_worker() -> Result<(Worker, PrimaryInit)> {
    let (job_sender, job_receiver) = mpsc::channel::<Job>();
    let (init_sender, init_receiver) = oneshot::channel();
    std::thread::Builder::new()
        .name("restart-manager".to_owned())
        .spawn(move || {
            let session = match crate::RestartSession::new() {
                Ok(session) => session,
                Err(error) => {
                    let _ = init_sender.send(Err(error));
                    return;
                }
            };
            let init = PrimaryInit {
                key: session.session_key().clone(),
                cancellation: session.cancellation_handle(),
            };
            if init_sender.send(Ok(init)).is_err() {
                return;
            }
            run_worker(job_receiver, WorkerState::Primary(session));
        })
        .map_err(|error| {
            Error::new(
                ErrorKind::AsyncWorkerUnavailable,
                error.raw_os_error().map(|code| code as u32),
                format!("the Restart Manager worker thread could not be created: {error}"),
            )
        })?;
    let init = init_receiver.await.map_err(|_| worker_unavailable())??;
    Ok((Worker { sender: job_sender }, init))
}

async fn start_joined_worker(key: SessionKey) -> Result<Worker> {
    let (job_sender, job_receiver) = mpsc::channel::<Job>();
    let (init_sender, init_receiver) = oneshot::channel();
    std::thread::Builder::new()
        .name("restart-manager-joined".to_owned())
        .spawn(move || {
            let session = match crate::JoinedSession::join(&key) {
                Ok(session) => session,
                Err(error) => {
                    let _ = init_sender.send(Err(error));
                    return;
                }
            };
            if init_sender.send(Ok(())).is_err() {
                return;
            }
            run_worker(job_receiver, WorkerState::Joined(session));
        })
        .map_err(|error| {
            Error::new(
                ErrorKind::AsyncWorkerUnavailable,
                error.raw_os_error().map(|code| code as u32),
                format!("the Restart Manager worker thread could not be created: {error}"),
            )
        })?;
    init_receiver.await.map_err(|_| worker_unavailable())??;
    Ok(Worker { sender: job_sender })
}

fn run_worker(receiver: mpsc::Receiver<Job>, mut state: WorkerState) {
    while let Ok(job) = receiver.recv() {
        job(&mut state);
    }
}

fn worker_unavailable() -> Error {
    Error::new(
        ErrorKind::AsyncWorkerUnavailable,
        None,
        "the dedicated Restart Manager worker is unavailable",
    )
}

fn wrong_state() -> Error {
    Error::new(
        ErrorKind::OperationOutOfSequence,
        None,
        "the asynchronous worker received an operation for the wrong typestate",
    )
}

/// Cloneable, coalescing receiver for asynchronous native progress.
#[derive(Clone)]
pub struct ProgressReceiver {
    receiver: watch::Receiver<Option<Progress>>,
}

impl ProgressReceiver {
    /// Peeks at the latest progress value without marking it as received.
    #[must_use]
    pub fn peek(&self) -> Option<Progress> {
        *self.receiver.borrow()
    }

    /// Receives a newer progress value, or `None` after normal operation end.
    pub async fn recv(&mut self) -> Option<Progress> {
        self.receiver.changed().await.ok()?;
        self.peek()
    }
}

/// A primary async Restart Manager session.
pub struct RestartSession {
    worker: Worker,
    key: SessionKey,
    cancellation: CancellationHandle,
}

impl RestartSession {
    /// Starts a new session on its dedicated worker thread.
    pub async fn new() -> Result<Self> {
        let (worker, init) = start_primary_worker().await?;
        Ok(Self {
            worker,
            key: init.key,
            cancellation: init.cancellation,
        })
    }

    /// Returns the cross-process session key.
    #[must_use]
    pub fn session_key(&self) -> &SessionKey {
        &self.key
    }

    /// Returns a cross-thread cancellation capability.
    #[must_use]
    pub fn cancellation_handle(&self) -> CancellationHandle {
        self.cancellation.clone()
    }

    /// Registers a resource batch in one worker command.
    pub async fn register_resources(&mut self, resources: &ResourceBatch) -> Result<()> {
        let resources = resources.clone();
        self.worker
            .call(move |state| match state {
                WorkerState::Primary(session) => session.register_resources(&resources),
                _ => Err(wrong_state()),
            })
            .await?
    }

    /// Registers file paths.
    pub async fn register_files<I, P>(&mut self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut resources = ResourceBatch::new();
        for file in files {
            resources.add_file(file.as_ref().to_path_buf());
        }
        self.register_resources(&resources).await
    }

    /// Registers exact process identities.
    pub async fn register_processes<I, P>(&mut self, processes: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: Borrow<ProcessIdentity>,
    {
        let mut resources = ResourceBatch::new();
        for process in processes {
            resources.add_process(*process.borrow());
        }
        self.register_resources(&resources).await
    }

    /// Registers service short names.
    pub async fn register_services<I, S>(&mut self, services: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut resources = ResourceBatch::new();
        for service in services {
            resources.add_service(service.as_ref().to_os_string());
        }
        self.register_resources(&resources).await
    }

    /// Takes an affected-application snapshot.
    pub async fn affected_applications(&mut self) -> Result<AffectedApplications> {
        self.worker
            .call(|state| match state {
                WorkerState::Primary(session) => session.affected_applications(),
                _ => Err(wrong_state()),
            })
            .await?
    }

    /// Adds or replaces a filter.
    pub async fn set_filter(&mut self, target: &FilterTarget, action: FilterAction) -> Result<()> {
        let target = target.clone();
        self.worker
            .call(move |state| match state {
                WorkerState::Primary(session) => session.set_filter(&target, action),
                _ => Err(wrong_state()),
            })
            .await?
    }

    /// Removes a filter.
    pub async fn remove_filter(&mut self, target: &FilterTarget) -> Result<()> {
        let target = target.clone();
        self.worker
            .call(move |state| match state {
                WorkerState::Primary(session) => session.remove_filter(&target),
                _ => Err(wrong_state()),
            })
            .await?
    }

    /// Lists filters.
    pub async fn filters(&mut self) -> Result<Vec<Filter>> {
        self.worker
            .call(|state| match state {
                WorkerState::Primary(session) => session.filters(),
                _ => Err(wrong_state()),
            })
            .await?
    }

    /// Starts a graceful shutdown future.
    #[must_use]
    pub fn shutdown(self) -> ShutdownFuture {
        self.shutdown_with_options(ShutdownOptions::default())
    }

    /// Starts shutdown with explicit options.
    #[must_use]
    pub fn shutdown_with_options(self, options: ShutdownOptions) -> ShutdownFuture {
        let Self {
            worker,
            key,
            cancellation,
        } = self;
        let (sender, receiver) = oneshot::channel();
        let _ = worker.send(Box::new(move |state| {
            let previous = std::mem::replace(state, WorkerState::Empty);
            match previous {
                WorkerState::Primary(session) => {
                    let pending = session.shutdown_with_options(options);
                    let outcome = pending.shutdown_outcome().clone();
                    *state = WorkerState::Pending(pending);
                    let _ = sender.send(ShutdownReply::Pending(outcome));
                }
                other => {
                    *state = other;
                    let _ = sender.send(ShutdownReply::NotStarted(wrong_state()));
                }
            }
        }));
        ShutdownFuture {
            worker: Some(worker),
            key,
            cancellation,
            receiver,
            cancel_on_drop: true,
        }
    }

    /// Starts shutdown and returns a coalescing progress receiver.
    #[must_use]
    pub fn shutdown_with_progress(
        self,
        options: ShutdownOptions,
    ) -> (ShutdownFuture, ProgressReceiver) {
        let Self {
            worker,
            key,
            cancellation,
        } = self;
        let (progress_sender, progress_receiver) = watch::channel(None);
        let (sender, receiver) = oneshot::channel();
        let _ = worker.send(Box::new(move |state| {
            let previous = std::mem::replace(state, WorkerState::Empty);
            match previous {
                WorkerState::Primary(session) => {
                    match session.shutdown_with_progress(options, |progress| {
                        progress_sender.send_replace(Some(progress));
                    }) {
                        Ok(pending) => {
                            let outcome = pending.shutdown_outcome().clone();
                            *state = WorkerState::Pending(pending);
                            let _ = sender.send(ShutdownReply::Pending(outcome));
                        }
                        Err(not_started) => {
                            let (session, error) = not_started.into_parts();
                            *state = WorkerState::Primary(session);
                            let _ = sender.send(ShutdownReply::NotStarted(error));
                        }
                    }
                }
                other => {
                    *state = other;
                    let _ = sender.send(ShutdownReply::NotStarted(wrong_state()));
                }
            }
        }));
        (
            ShutdownFuture {
                worker: Some(worker),
                key,
                cancellation,
                receiver,
                cancel_on_drop: true,
            },
            ProgressReceiver {
                receiver: progress_receiver,
            },
        )
    }

    /// Ends the session without shutdown.
    pub async fn end(self) -> Result<()> {
        let Self { worker, .. } = self;
        worker
            .call(|state| {
                let previous = std::mem::replace(state, WorkerState::Empty);
                match previous {
                    WorkerState::Primary(session) => session.end(),
                    other => {
                        *state = other;
                        Err(wrong_state())
                    }
                }
            })
            .await?
    }
}

enum ShutdownReply {
    Pending(OperationOutcome),
    NotStarted(Error),
}

/// Future for an asynchronous shutdown attempt.
///
/// Dropping it requests best-effort cancellation. The worker still owns the
/// session and will restart anything partially stopped before ending.
pub struct ShutdownFuture {
    worker: Option<Worker>,
    key: SessionKey,
    cancellation: CancellationHandle,
    receiver: oneshot::Receiver<ShutdownReply>,
    cancel_on_drop: bool,
}

impl Future for ShutdownFuture {
    type Output = std::result::Result<RestartPending, OperationNotStarted<RestartSession>>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match Pin::new(&mut this.receiver).poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(ShutdownReply::Pending(shutdown))) => {
                this.cancel_on_drop = false;
                Poll::Ready(Ok(RestartPending {
                    worker: this.worker.take().expect("future owns its worker"),
                    shutdown,
                }))
            }
            Poll::Ready(Ok(ShutdownReply::NotStarted(error))) => {
                this.cancel_on_drop = false;
                let state = RestartSession {
                    worker: this.worker.take().expect("future owns its worker"),
                    key: this.key.clone(),
                    cancellation: this.cancellation.clone(),
                };
                Poll::Ready(Err(OperationNotStarted::new(state, error)))
            }
            Poll::Ready(Err(_)) => {
                this.cancel_on_drop = false;
                let state = RestartSession {
                    worker: this.worker.take().expect("future owns its worker"),
                    key: this.key.clone(),
                    cancellation: this.cancellation.clone(),
                };
                Poll::Ready(Err(OperationNotStarted::new(state, worker_unavailable())))
            }
        }
    }
}

impl Drop for ShutdownFuture {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            let _ = self.cancellation.cancel();
        }
    }
}

/// Async recovery-required state after shutdown.
#[must_use = "call restart or leave_stopped explicitly"]
pub struct RestartPending {
    worker: Worker,
    shutdown: OperationOutcome,
}

impl RestartPending {
    /// Returns the retained shutdown result.
    #[must_use]
    pub const fn shutdown_outcome(&self) -> &OperationOutcome {
        &self.shutdown
    }

    /// Starts a restart future.
    #[must_use]
    pub fn restart(self) -> RestartFuture {
        let Self {
            worker,
            shutdown: _,
        } = self;
        let (sender, receiver) = oneshot::channel();
        let _ = worker.send(Box::new(move |state| {
            let previous = std::mem::replace(state, WorkerState::Empty);
            match previous {
                WorkerState::Pending(pending) => {
                    let completion = pending.restart();
                    let outcome = completion.outcome().clone();
                    *state = WorkerState::Completion(completion);
                    let _ = sender.send(Ok(outcome));
                }
                other => {
                    *state = other;
                    let _ = sender.send(Err(wrong_state()));
                }
            }
        }));
        RestartFuture {
            worker: Some(worker),
            receiver,
        }
    }

    /// Starts restart and returns its coalescing progress receiver.
    #[must_use]
    pub fn restart_with_progress(self) -> (RestartWithProgressFuture, ProgressReceiver) {
        let Self { worker, shutdown } = self;
        let (progress_sender, progress_receiver) = watch::channel(None);
        let (sender, receiver) = oneshot::channel();
        let _ = worker.send(Box::new(move |state| {
            let previous = std::mem::replace(state, WorkerState::Empty);
            match previous {
                WorkerState::Pending(pending) => {
                    match pending.restart_with_progress(|progress| {
                        progress_sender.send_replace(Some(progress));
                    }) {
                        Ok(completion) => {
                            let outcome = completion.outcome().clone();
                            *state = WorkerState::Completion(completion);
                            let _ = sender.send(RestartProgressReply::Completed(outcome));
                        }
                        Err(not_started) => {
                            let (pending, error) = not_started.into_parts();
                            *state = WorkerState::Pending(pending);
                            let _ = sender.send(RestartProgressReply::NotStarted(error));
                        }
                    }
                }
                other => {
                    *state = other;
                    let _ = sender.send(RestartProgressReply::NotStarted(wrong_state()));
                }
            }
        }));
        (
            RestartWithProgressFuture {
                worker: Some(worker),
                shutdown,
                receiver,
            },
            ProgressReceiver {
                receiver: progress_receiver,
            },
        )
    }

    /// Explicitly opts out of restart.
    pub async fn leave_stopped(self) -> Result<RecoveryCompletion> {
        let Self { worker, .. } = self;
        let outcome = worker
            .call(|state| {
                let previous = std::mem::replace(state, WorkerState::Empty);
                match previous {
                    WorkerState::Pending(pending) => {
                        let completion = pending.leave_stopped();
                        let outcome = completion.outcome().clone();
                        *state = WorkerState::Completion(completion);
                        Ok(outcome)
                    }
                    other => {
                        *state = other;
                        Err(wrong_state())
                    }
                }
            })
            .await??;
        Ok(RecoveryCompletion { worker, outcome })
    }
}

/// Future for restart without progress.
///
/// Dropping this future detaches the operation; the worker completes restart
/// and ends the session.
pub struct RestartFuture {
    worker: Option<Worker>,
    receiver: oneshot::Receiver<Result<RecoveryOutcome>>,
}

impl Future for RestartFuture {
    type Output = Result<RecoveryCompletion>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match Pin::new(&mut this.receiver).poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(Ok(outcome))) => Poll::Ready(Ok(RecoveryCompletion {
                worker: this.worker.take().expect("future owns its worker"),
                outcome,
            })),
            Poll::Ready(Ok(Err(error))) => Poll::Ready(Err(error)),
            Poll::Ready(Err(_)) => Poll::Ready(Err(worker_unavailable())),
        }
    }
}

enum RestartProgressReply {
    Completed(RecoveryOutcome),
    NotStarted(Error),
}

/// Future for restart with coalescing progress.
pub struct RestartWithProgressFuture {
    worker: Option<Worker>,
    shutdown: OperationOutcome,
    receiver: oneshot::Receiver<RestartProgressReply>,
}

impl Future for RestartWithProgressFuture {
    type Output = std::result::Result<RecoveryCompletion, OperationNotStarted<RestartPending>>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match Pin::new(&mut this.receiver).poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(RestartProgressReply::Completed(outcome))) => {
                Poll::Ready(Ok(RecoveryCompletion {
                    worker: this.worker.take().expect("future owns its worker"),
                    outcome,
                }))
            }
            Poll::Ready(Ok(RestartProgressReply::NotStarted(error))) => {
                let state = RestartPending {
                    worker: this.worker.take().expect("future owns its worker"),
                    shutdown: this.shutdown.clone(),
                };
                Poll::Ready(Err(OperationNotStarted::new(state, error)))
            }
            Poll::Ready(Err(_)) => {
                let state = RestartPending {
                    worker: this.worker.take().expect("future owns its worker"),
                    shutdown: this.shutdown.clone(),
                };
                Poll::Ready(Err(OperationNotStarted::new(state, worker_unavailable())))
            }
        }
    }
}

/// Async completed-recovery state.
pub struct RecoveryCompletion {
    worker: Worker,
    outcome: RecoveryOutcome,
}

impl RecoveryCompletion {
    /// Returns both retained outcomes.
    #[must_use]
    pub const fn outcome(&self) -> &RecoveryOutcome {
        &self.outcome
    }

    /// Takes a post-operation report.
    pub async fn affected_applications(&mut self) -> Result<AffectedApplications> {
        self.worker
            .call(|state| match state {
                WorkerState::Completion(completion) => completion.affected_applications(),
                _ => Err(wrong_state()),
            })
            .await?
    }

    /// Ends the worker-owned session and returns both outcomes.
    pub async fn end(self) -> Result<RecoveryOutcome> {
        let Self { worker, .. } = self;
        worker
            .call(|state| {
                let previous = std::mem::replace(state, WorkerState::Empty);
                match previous {
                    WorkerState::Completion(completion) => completion.end(),
                    other => {
                        *state = other;
                        Err(wrong_state())
                    }
                }
            })
            .await?
    }
}

/// An async secondary-installer session.
pub struct JoinedSession {
    worker: Worker,
    key: SessionKey,
}

impl JoinedSession {
    /// Joins an existing session on a dedicated worker.
    pub async fn join(key: &SessionKey) -> Result<Self> {
        let key = key.clone();
        let worker = start_joined_worker(key.clone()).await?;
        Ok(Self { worker, key })
    }

    /// Returns the joined key.
    #[must_use]
    pub fn session_key(&self) -> &SessionKey {
        &self.key
    }

    /// Registers a resource batch.
    pub async fn register_resources(&mut self, resources: &ResourceBatch) -> Result<()> {
        let resources = resources.clone();
        self.worker
            .call(move |state| match state {
                WorkerState::Joined(session) => session.register_resources(&resources),
                _ => Err(wrong_state()),
            })
            .await?
    }

    /// Registers file paths.
    pub async fn register_files<I, P>(&mut self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut resources = ResourceBatch::new();
        for file in files {
            resources.add_file(file.as_ref().to_path_buf());
        }
        self.register_resources(&resources).await
    }

    /// Registers process identities.
    pub async fn register_processes<I, P>(&mut self, processes: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: Borrow<ProcessIdentity>,
    {
        let mut resources = ResourceBatch::new();
        for process in processes {
            resources.add_process(*process.borrow());
        }
        self.register_resources(&resources).await
    }

    /// Registers service short names.
    pub async fn register_services<I, S>(&mut self, services: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut resources = ResourceBatch::new();
        for service in services {
            resources.add_service(service.as_ref().to_os_string());
        }
        self.register_resources(&resources).await
    }

    /// Ends the joined handle.
    pub async fn end(self) -> Result<()> {
        let Self { worker, .. } = self;
        worker
            .call(|state| {
                let previous = std::mem::replace(state, WorkerState::Empty);
                match previous {
                    WorkerState::Joined(session) => session.end(),
                    other => {
                        *state = other;
                        Err(wrong_state())
                    }
                }
            })
            .await?
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::sync::Arc;
    use std::task::{Wake, Waker};

    use super::*;

    struct ThreadWake(std::thread::Thread);

    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::park(),
            }
        }
    }

    #[test]
    fn worker_owns_the_complete_typestate_sequence() {
        let mut session = block_on(RestartSession::new()).unwrap();
        block_on(session.register_resources(&ResourceBatch::new())).unwrap();
        let pending = block_on(session.shutdown()).unwrap();
        assert!(pending.shutdown_outcome().is_success());
        let completion = block_on(pending.restart()).unwrap();
        assert!(completion.outcome().shutdown_outcome().is_success());
        assert!(
            completion
                .outcome()
                .restart_outcome()
                .is_some_and(OperationOutcome::is_success)
        );
        block_on(completion.end()).unwrap();
    }

    #[test]
    fn progress_receiver_coalesces_and_peek_does_not_consume() {
        let (sender, receiver) = watch::channel(None);
        let mut receiver = ProgressReceiver { receiver };
        sender.send_replace(Progress::try_from_native(10));
        sender.send_replace(Progress::try_from_native(20));
        assert_eq!(receiver.peek().unwrap().percent_complete(), 20);
        assert_eq!(receiver.peek().unwrap().percent_complete(), 20);
        assert_eq!(block_on(receiver.recv()).unwrap().percent_complete(), 20);
    }

    #[test]
    fn progress_receiver_recv_wakes_and_closes_normally() {
        let (sender, receiver) = watch::channel(None);
        let mut receiver = ProgressReceiver { receiver };
        let producer = std::thread::spawn(move || {
            sender.send_replace(Progress::try_from_native(30));
        });
        assert_eq!(block_on(receiver.recv()).unwrap().percent_complete(), 30);
        producer.join().unwrap();
        assert_eq!(block_on(receiver.recv()), None);
    }

    #[test]
    fn worker_failure_remains_on_the_operation_future() {
        let session = crate::RestartSession::new().unwrap();
        let key = session.session_key().clone();
        let cancellation = session.cancellation_handle();
        session.end().unwrap();

        let (job_sender, job_receiver) = mpsc::channel();
        drop(job_receiver);
        let worker = Worker { sender: job_sender };
        let (reply_sender, receiver) = oneshot::channel();
        drop(reply_sender);
        let future = ShutdownFuture {
            worker: Some(worker),
            key,
            cancellation,
            receiver,
            cancel_on_drop: true,
        };
        let error = match block_on(future) {
            Ok(_) => panic!("a disconnected shutdown reply unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.error().kind(), ErrorKind::AsyncWorkerUnavailable);

        let (job_sender, job_receiver) = mpsc::channel();
        drop(job_receiver);
        let worker = Worker { sender: job_sender };
        let (reply_sender, receiver) = oneshot::channel();
        drop(reply_sender);
        let result = block_on(RestartFuture {
            worker: Some(worker),
            receiver,
        });
        let error = match result {
            Ok(_) => panic!("a disconnected restart reply unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), ErrorKind::AsyncWorkerUnavailable);
    }

    #[test]
    fn primary_and_joined_workers_cover_every_registration_and_filter_command() {
        let mut primary = block_on(RestartSession::new()).unwrap();
        let key = primary.session_key().clone();
        let cancellation = primary.cancellation_handle();
        let process = ProcessIdentity::current().unwrap();
        let executable = std::env::current_exe().unwrap();

        let mut joined = block_on(JoinedSession::join(&key)).unwrap();
        assert_eq!(joined.session_key(), &key);
        block_on(joined.register_resources(&ResourceBatch::new())).unwrap();
        block_on(joined.register_files([&executable])).unwrap();
        block_on(joined.register_processes([process])).unwrap();
        block_on(joined.register_services(["EventLog"])).unwrap();
        block_on(joined.end()).unwrap();

        block_on(primary.register_files([&executable])).unwrap();
        block_on(primary.register_processes([process])).unwrap();
        block_on(primary.register_services(["EventLog"])).unwrap();
        assert!(
            !block_on(primary.affected_applications())
                .unwrap()
                .is_empty()
        );

        let process_target = FilterTarget::process(process);
        let service_target = FilterTarget::service("EventLog").unwrap();
        block_on(primary.set_filter(&process_target, FilterAction::PreventRestart)).unwrap();
        block_on(primary.set_filter(&service_target, FilterAction::PreventShutdown)).unwrap();
        let filters = block_on(primary.filters()).unwrap();
        assert!(
            filters
                .iter()
                .any(|filter| filter.target() == &process_target)
        );
        assert!(
            filters
                .iter()
                .any(|filter| filter.target() == &service_target)
        );
        block_on(primary.remove_filter(&process_target)).unwrap();
        block_on(primary.remove_filter(&service_target)).unwrap();
        block_on(primary.end()).unwrap();
        assert_eq!(
            cancellation.cancel().unwrap_err().kind(),
            ErrorKind::SessionEnded
        );
    }

    #[test]
    fn progress_futures_and_leave_stopped_preserve_outcomes() {
        let _test_guard = crate::sys::serialize_callback_test();
        let mut session = block_on(RestartSession::new()).unwrap();
        let process = ProcessIdentity::current().unwrap();
        block_on(session.register_processes([process])).unwrap();
        block_on(session.set_filter(
            &FilterTarget::process(process),
            FilterAction::PreventShutdown,
        ))
        .unwrap();
        let (shutdown, progress) = session.shutdown_with_progress(ShutdownOptions::default());
        let pending = block_on(shutdown).unwrap();
        assert!(progress.peek().is_some());
        let (restart, progress) = pending.restart_with_progress();
        let mut completion = block_on(restart).unwrap();
        let _ = progress.peek();
        block_on(completion.affected_applications()).unwrap();
        block_on(completion.end()).unwrap();

        let session = block_on(RestartSession::new()).unwrap();
        let pending = block_on(session.shutdown()).unwrap();
        let completion = block_on(pending.leave_stopped()).unwrap();
        assert!(completion.outcome().restart_outcome().is_none());
        block_on(completion.end()).unwrap();
    }

    #[test]
    fn dropping_restart_futures_detaches_worker_cleanup() {
        let _test_guard = crate::sys::serialize_callback_test();
        let session = block_on(RestartSession::new()).unwrap();
        let pending = block_on(session.shutdown()).unwrap();
        drop(pending.restart());

        let session = block_on(RestartSession::new()).unwrap();
        let pending = block_on(session.shutdown()).unwrap();
        let (restart, _progress) = pending.restart_with_progress();
        drop(restart);

        std::thread::sleep(std::time::Duration::from_millis(100));
        crate::RestartSession::new().unwrap().end().unwrap();
    }

    #[test]
    fn dropping_shutdown_future_leaves_cleanup_with_the_worker() {
        let session = block_on(RestartSession::new()).unwrap();
        drop(session.shutdown());
        std::thread::sleep(std::time::Duration::from_millis(100));
        crate::RestartSession::new().unwrap().end().unwrap();
    }
}
