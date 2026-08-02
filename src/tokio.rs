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
use std::fmt;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::mpsc;
use std::task::{Context, Poll};

use ::tokio::sync::{oneshot, watch};

use crate::{
    AffectedApplications, CancellationHandle, Error, ErrorKind, Filter, FilterAction, FilterTarget,
    OperationOutcome, ProcessIdentity, Progress, RecoveryOutcome, ResourceBatch, Result,
    SessionKey, ShutdownOptions,
};

enum WorkerState {
    Primary(crate::RestartSession),
    Joined(crate::JoinedSession),
    Pending(crate::RestartPending),
    Completion(crate::RecoveryCompletion),
    Empty,
}

enum Command {
    PrimaryRegister {
        resources: ResourceBatch,
        reply: oneshot::Sender<Result<()>>,
    },
    PrimaryAffected {
        reply: oneshot::Sender<Result<AffectedApplications>>,
    },
    SetFilter {
        target: FilterTarget,
        action: FilterAction,
        reply: oneshot::Sender<Result<()>>,
    },
    RemoveFilter {
        target: FilterTarget,
        reply: oneshot::Sender<Result<()>>,
    },
    Filters {
        reply: oneshot::Sender<Result<Vec<Filter>>>,
    },
    Shutdown {
        options: ShutdownOptions,
        reply: oneshot::Sender<ShutdownReply>,
    },
    ShutdownWithProgress {
        options: ShutdownOptions,
        progress: watch::Sender<Option<Progress>>,
        reply: oneshot::Sender<ShutdownReply>,
    },
    EndPrimary {
        reply: oneshot::Sender<Result<()>>,
    },
    Restart {
        reply: oneshot::Sender<Result<RecoveryOutcome>>,
    },
    RestartWithProgress {
        progress: watch::Sender<Option<Progress>>,
        reply: oneshot::Sender<RestartProgressReply>,
    },
    LeaveStopped {
        reply: oneshot::Sender<Result<RecoveryOutcome>>,
    },
    CompletionAffected {
        reply: oneshot::Sender<Result<AffectedApplications>>,
    },
    EndCompletion {
        reply: oneshot::Sender<Result<RecoveryOutcome>>,
    },
    JoinedRegister {
        resources: ResourceBatch,
        reply: oneshot::Sender<Result<()>>,
    },
    EndJoined {
        reply: oneshot::Sender<Result<()>>,
    },
}

#[cfg(all(test, windows))]
#[derive(Clone)]
struct WorkerExitNotification {
    state: std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
}

#[cfg(all(test, windows))]
impl WorkerExitNotification {
    fn new() -> Self {
        Self {
            state: std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new())),
        }
    }

    fn notify(&self) {
        let (stopped, changed) = &*self.state;
        *stopped.lock().unwrap_or_else(|error| error.into_inner()) = true;
        changed.notify_all();
    }

    fn wait(&self) {
        let (stopped, changed) = &*self.state;
        let mut stopped = stopped.lock().unwrap_or_else(|error| error.into_inner());
        while !*stopped {
            stopped = changed
                .wait(stopped)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

#[cfg(all(test, windows))]
struct WorkerExitGuard(WorkerExitNotification);

#[cfg(all(test, windows))]
impl Drop for WorkerExitGuard {
    fn drop(&mut self) {
        self.0.notify();
    }
}

struct Worker {
    sender: mpsc::Sender<Command>,
    #[cfg(all(test, windows))]
    exit: WorkerExitNotification,
}

impl Worker {
    fn send(&self, command: Command) -> Result<()> {
        self.sender.send(command).map_err(|_| worker_unavailable())
    }

    #[cfg(all(test, windows))]
    fn exit_notification(&self) -> WorkerExitNotification {
        self.exit.clone()
    }
}

struct PrimaryInit {
    key: SessionKey,
    cancellation: CancellationHandle,
}

async fn start_primary_worker() -> Result<(Worker, PrimaryInit)> {
    let (command_sender, command_receiver) = mpsc::channel::<Command>();
    let (init_sender, init_receiver) = oneshot::channel();
    #[cfg(all(test, windows))]
    let exit = WorkerExitNotification::new();
    #[cfg(all(test, windows))]
    let thread_exit = exit.clone();
    std::thread::Builder::new()
        .name("restart-manager".to_owned())
        .spawn(move || {
            #[cfg(all(test, windows))]
            let _exit_guard = WorkerExitGuard(thread_exit);
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
            run_worker(command_receiver, WorkerState::Primary(session));
        })
        .map_err(|error| {
            Error::new(
                ErrorKind::AsyncWorkerUnavailable,
                error.raw_os_error().map(|code| code as u32),
                format!("the Restart Manager worker thread could not be created: {error}"),
            )
        })?;
    let init = init_receiver.await.map_err(|_| worker_unavailable())??;
    Ok((
        Worker {
            sender: command_sender,
            #[cfg(all(test, windows))]
            exit,
        },
        init,
    ))
}

async fn start_joined_worker(key: SessionKey) -> Result<Worker> {
    let (command_sender, command_receiver) = mpsc::channel::<Command>();
    let (init_sender, init_receiver) = oneshot::channel();
    #[cfg(all(test, windows))]
    let exit = WorkerExitNotification::new();
    #[cfg(all(test, windows))]
    let thread_exit = exit.clone();
    std::thread::Builder::new()
        .name("restart-manager-joined".to_owned())
        .spawn(move || {
            #[cfg(all(test, windows))]
            let _exit_guard = WorkerExitGuard(thread_exit);
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
            run_worker(command_receiver, WorkerState::Joined(session));
        })
        .map_err(|error| {
            Error::new(
                ErrorKind::AsyncWorkerUnavailable,
                error.raw_os_error().map(|code| code as u32),
                format!("the Restart Manager worker thread could not be created: {error}"),
            )
        })?;
    init_receiver.await.map_err(|_| worker_unavailable())??;
    Ok(Worker {
        sender: command_sender,
        #[cfg(all(test, windows))]
        exit,
    })
}

fn run_worker(receiver: mpsc::Receiver<Command>, mut state: WorkerState) {
    while let Ok(command) = receiver.recv() {
        execute_command(command, &mut state);
    }
}

fn execute_command(command: Command, state: &mut WorkerState) {
    match command {
        Command::PrimaryRegister { resources, reply } => {
            let result = match state {
                WorkerState::Primary(session) => session.register_resources(&resources),
                _ => Err(wrong_state()),
            };
            let _ = reply.send(result);
        }
        Command::PrimaryAffected { reply } => {
            let result = match state {
                WorkerState::Primary(session) => session.affected_applications(),
                _ => Err(wrong_state()),
            };
            let _ = reply.send(result);
        }
        Command::SetFilter {
            target,
            action,
            reply,
        } => {
            let result = match state {
                WorkerState::Primary(session) => session.set_filter(&target, action),
                _ => Err(wrong_state()),
            };
            let _ = reply.send(result);
        }
        Command::RemoveFilter { target, reply } => {
            let result = match state {
                WorkerState::Primary(session) => session.remove_filter(&target),
                _ => Err(wrong_state()),
            };
            let _ = reply.send(result);
        }
        Command::Filters { reply } => {
            let result = match state {
                WorkerState::Primary(session) => session.filters(),
                _ => Err(wrong_state()),
            };
            let _ = reply.send(result);
        }
        Command::Shutdown { options, reply } => {
            let previous = std::mem::replace(state, WorkerState::Empty);
            let result = match previous {
                WorkerState::Primary(session) => {
                    let pending = session.shutdown_with_options(options);
                    let outcome = pending.shutdown_outcome().clone();
                    *state = WorkerState::Pending(pending);
                    ShutdownReply::Pending(outcome)
                }
                other => {
                    *state = other;
                    ShutdownReply::Failed(wrong_state())
                }
            };
            let _ = reply.send(result);
        }
        Command::ShutdownWithProgress {
            options,
            progress,
            reply,
        } => {
            let previous = std::mem::replace(state, WorkerState::Empty);
            let result = match previous {
                WorkerState::Primary(session) => {
                    match session.shutdown_with_progress(options, |value| {
                        progress.send_replace(Some(value));
                    }) {
                        Ok(pending) => {
                            let outcome = pending.shutdown_outcome().clone();
                            *state = WorkerState::Pending(pending);
                            ShutdownReply::Pending(outcome)
                        }
                        Err(not_started) => {
                            let (session, error) = not_started.into_parts();
                            *state = WorkerState::Primary(session);
                            ShutdownReply::Recoverable(error)
                        }
                    }
                }
                other => {
                    *state = other;
                    ShutdownReply::Failed(wrong_state())
                }
            };
            let _ = reply.send(result);
        }
        Command::EndPrimary { reply } => {
            let previous = std::mem::replace(state, WorkerState::Empty);
            let result = match previous {
                WorkerState::Primary(session) => session.end(),
                other => {
                    *state = other;
                    Err(wrong_state())
                }
            };
            let _ = reply.send(result);
        }
        Command::Restart { reply } => {
            let previous = std::mem::replace(state, WorkerState::Empty);
            let result = match previous {
                WorkerState::Pending(pending) => {
                    let completion = pending.restart();
                    let outcome = completion.outcome().clone();
                    *state = WorkerState::Completion(completion);
                    Ok(outcome)
                }
                other => {
                    *state = other;
                    Err(wrong_state())
                }
            };
            let _ = reply.send(result);
        }
        Command::RestartWithProgress { progress, reply } => {
            let previous = std::mem::replace(state, WorkerState::Empty);
            let result = match previous {
                WorkerState::Pending(pending) => {
                    match pending.restart_with_progress(|value| {
                        progress.send_replace(Some(value));
                    }) {
                        Ok(completion) => {
                            let outcome = completion.outcome().clone();
                            *state = WorkerState::Completion(completion);
                            RestartProgressReply::Completed(outcome)
                        }
                        Err(not_started) => {
                            let (pending, error) = not_started.into_parts();
                            *state = WorkerState::Pending(pending);
                            RestartProgressReply::Recoverable(error)
                        }
                    }
                }
                other => {
                    *state = other;
                    RestartProgressReply::Failed(wrong_state())
                }
            };
            let _ = reply.send(result);
        }
        Command::LeaveStopped { reply } => {
            let previous = std::mem::replace(state, WorkerState::Empty);
            let result = match previous {
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
            };
            let _ = reply.send(result);
        }
        Command::CompletionAffected { reply } => {
            let result = match state {
                WorkerState::Completion(completion) => completion.affected_applications(),
                _ => Err(wrong_state()),
            };
            let _ = reply.send(result);
        }
        Command::EndCompletion { reply } => {
            let previous = std::mem::replace(state, WorkerState::Empty);
            let result = match previous {
                WorkerState::Completion(completion) => completion.end(),
                other => {
                    *state = other;
                    Err(wrong_state())
                }
            };
            let _ = reply.send(result);
        }
        Command::JoinedRegister { resources, reply } => {
            let result = match state {
                WorkerState::Joined(session) => session.register_resources(&resources),
                _ => Err(wrong_state()),
            };
            let _ = reply.send(result);
        }
        Command::EndJoined { reply } => {
            let previous = std::mem::replace(state, WorkerState::Empty);
            let result = match previous {
                WorkerState::Joined(session) => session.end(),
                other => {
                    *state = other;
                    Err(wrong_state())
                }
            };
            let _ = reply.send(result);
        }
    }
}

async fn receive<T>(receiver: oneshot::Receiver<T>) -> Result<T> {
    receiver.await.map_err(|_| worker_unavailable())
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

/// Error from an async consuming operation.
///
/// A callback-lease conflict occurs before native work and retains the
/// reusable typestate. Worker failure or an internal state mismatch returns no
/// state because the facade cannot prove that retrying it would be sound.
pub struct AsyncOperationError<T> {
    state: Option<T>,
    error: Error,
}

impl<T> AsyncOperationError<T> {
    fn recoverable(state: T, error: Error) -> Self {
        Self {
            state: Some(state),
            error,
        }
    }

    fn unavailable(error: Error) -> Self {
        Self { state: None, error }
    }

    /// Returns the operation error.
    #[must_use]
    pub const fn error(&self) -> &Error {
        &self.error
    }

    /// Returns reusable state only when native work is known not to have begun.
    #[must_use]
    pub const fn state(&self) -> Option<&T> {
        self.state.as_ref()
    }

    /// Separates the optional reusable state and operation error.
    #[must_use]
    pub fn into_parts(self) -> (Option<T>, Error) {
        (self.state, self.error)
    }
}

impl<T> fmt::Debug for AsyncOperationError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AsyncOperationError")
            .field(
                "state",
                &self.state.as_ref().map(|_| std::any::type_name::<T>()),
            )
            .field("error", &self.error)
            .finish()
    }
}

impl<T> fmt::Display for AsyncOperationError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl<T> std::error::Error for AsyncOperationError<T> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
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
        let (reply, receiver) = oneshot::channel();
        self.worker.send(Command::PrimaryRegister {
            resources: resources.clone(),
            reply,
        })?;
        receive(receiver).await?
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
        let (reply, receiver) = oneshot::channel();
        self.worker.send(Command::PrimaryAffected { reply })?;
        receive(receiver).await?
    }

    /// Adds or replaces a filter.
    pub async fn set_filter(&mut self, target: &FilterTarget, action: FilterAction) -> Result<()> {
        let (reply, receiver) = oneshot::channel();
        self.worker.send(Command::SetFilter {
            target: target.clone(),
            action,
            reply,
        })?;
        receive(receiver).await?
    }

    /// Removes a filter.
    pub async fn remove_filter(&mut self, target: &FilterTarget) -> Result<()> {
        let (reply, receiver) = oneshot::channel();
        self.worker.send(Command::RemoveFilter {
            target: target.clone(),
            reply,
        })?;
        receive(receiver).await?
    }

    /// Lists filters.
    pub async fn filters(&mut self) -> Result<Vec<Filter>> {
        let (reply, receiver) = oneshot::channel();
        self.worker.send(Command::Filters { reply })?;
        receive(receiver).await?
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
        let (reply, receiver) = oneshot::channel();
        let _ = worker.send(Command::Shutdown { options, reply });
        ShutdownFuture {
            worker: Some(worker),
            key: Some(key),
            cancellation: Some(cancellation),
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
        let (reply, receiver) = oneshot::channel();
        let _ = worker.send(Command::ShutdownWithProgress {
            options,
            progress: progress_sender,
            reply,
        });
        (
            ShutdownFuture {
                worker: Some(worker),
                key: Some(key),
                cancellation: Some(cancellation),
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
        let (reply, receiver) = oneshot::channel();
        worker.send(Command::EndPrimary { reply })?;
        receive(receiver).await?
    }
}

enum ShutdownReply {
    Pending(OperationOutcome),
    Recoverable(Error),
    Failed(Error),
}

/// Future for an asynchronous shutdown attempt.
///
/// Dropping it requests best-effort cancellation. The worker still owns the
/// session and will restart anything partially stopped before ending.
pub struct ShutdownFuture {
    worker: Option<Worker>,
    key: Option<SessionKey>,
    cancellation: Option<CancellationHandle>,
    receiver: oneshot::Receiver<ShutdownReply>,
    cancel_on_drop: bool,
}

impl Future for ShutdownFuture {
    type Output = std::result::Result<RestartPending, AsyncOperationError<RestartSession>>;

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
            Poll::Ready(Ok(ShutdownReply::Recoverable(error))) => {
                this.cancel_on_drop = false;
                let state = RestartSession {
                    worker: this.worker.take().expect("future owns its worker"),
                    key: this.key.take().expect("future owns its session key"),
                    cancellation: this
                        .cancellation
                        .take()
                        .expect("future owns its cancellation handle"),
                };
                Poll::Ready(Err(AsyncOperationError::recoverable(state, error)))
            }
            Poll::Ready(Ok(ShutdownReply::Failed(error))) => {
                this.cancel_on_drop = false;
                Poll::Ready(Err(AsyncOperationError::unavailable(error)))
            }
            Poll::Ready(Err(_)) => {
                this.cancel_on_drop = false;
                Poll::Ready(Err(AsyncOperationError::unavailable(worker_unavailable())))
            }
        }
    }
}

impl Drop for ShutdownFuture {
    fn drop(&mut self) {
        if self.cancel_on_drop
            && let Some(cancellation) = &self.cancellation
        {
            let _ = cancellation.cancel();
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
        let (reply, receiver) = oneshot::channel();
        let _ = worker.send(Command::Restart { reply });
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
        let (reply, receiver) = oneshot::channel();
        let _ = worker.send(Command::RestartWithProgress {
            progress: progress_sender,
            reply,
        });
        (
            RestartWithProgressFuture {
                worker: Some(worker),
                shutdown: Some(shutdown),
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
        let (reply, receiver) = oneshot::channel();
        worker.send(Command::LeaveStopped { reply })?;
        let outcome = receive(receiver).await??;
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
    Recoverable(Error),
    Failed(Error),
}

/// Future for restart with coalescing progress.
pub struct RestartWithProgressFuture {
    worker: Option<Worker>,
    shutdown: Option<OperationOutcome>,
    receiver: oneshot::Receiver<RestartProgressReply>,
}

impl Future for RestartWithProgressFuture {
    type Output = std::result::Result<RecoveryCompletion, AsyncOperationError<RestartPending>>;

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
            Poll::Ready(Ok(RestartProgressReply::Recoverable(error))) => {
                let state = RestartPending {
                    worker: this.worker.take().expect("future owns its worker"),
                    shutdown: this
                        .shutdown
                        .take()
                        .expect("future owns its shutdown outcome"),
                };
                Poll::Ready(Err(AsyncOperationError::recoverable(state, error)))
            }
            Poll::Ready(Ok(RestartProgressReply::Failed(error))) => {
                Poll::Ready(Err(AsyncOperationError::unavailable(error)))
            }
            Poll::Ready(Err(_)) => {
                Poll::Ready(Err(AsyncOperationError::unavailable(worker_unavailable())))
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
        let (reply, receiver) = oneshot::channel();
        self.worker.send(Command::CompletionAffected { reply })?;
        receive(receiver).await?
    }

    /// Ends the worker-owned session and returns both outcomes.
    pub async fn end(self) -> Result<RecoveryOutcome> {
        let Self { worker, .. } = self;
        let (reply, receiver) = oneshot::channel();
        worker.send(Command::EndCompletion { reply })?;
        receive(receiver).await?
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
        let (reply, receiver) = oneshot::channel();
        self.worker.send(Command::JoinedRegister {
            resources: resources.clone(),
            reply,
        })?;
        receive(receiver).await?
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
        let (reply, receiver) = oneshot::channel();
        worker.send(Command::EndJoined { reply })?;
        receive(receiver).await?
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

    fn disconnected_worker() -> Worker {
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        Worker {
            sender,
            exit: WorkerExitNotification::new(),
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

        let (reply, _receiver) = oneshot::channel();
        assert_eq!(
            disconnected_worker()
                .send(Command::Filters { reply })
                .unwrap_err()
                .kind(),
            ErrorKind::AsyncWorkerUnavailable
        );

        let worker = disconnected_worker();
        let (reply_sender, receiver) = oneshot::channel();
        drop(reply_sender);
        let future = ShutdownFuture {
            worker: Some(worker),
            key: Some(key),
            cancellation: Some(cancellation),
            receiver,
            cancel_on_drop: true,
        };
        let error = match block_on(future) {
            Ok(_) => panic!("a disconnected shutdown reply unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.error().kind(), ErrorKind::AsyncWorkerUnavailable);
        assert!(error.state().is_none());
        assert!(format!("{error:?}").contains("AsyncOperationError"));
        assert_eq!(error.to_string(), error.error().to_string());
        assert!(std::error::Error::source(&error).is_some());

        let worker = disconnected_worker();
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

        let worker = disconnected_worker();
        let (reply_sender, receiver) = oneshot::channel();
        drop(reply_sender);
        let result = block_on(RestartWithProgressFuture {
            worker: Some(worker),
            shutdown: Some(OperationOutcome::Succeeded),
            receiver,
        });
        let error = match result {
            Ok(_) => panic!("a disconnected restart-progress reply unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.error().kind(), ErrorKind::AsyncWorkerUnavailable);
        assert!(error.state().is_none());
    }

    #[test]
    fn callback_conflicts_retain_only_provably_reusable_async_state() {
        let session = block_on(RestartSession::new()).unwrap();
        let error = crate::sys::with_callback_lease_for_test(|| {
            let (shutdown, _progress) = session.shutdown_with_progress(ShutdownOptions::default());
            match block_on(shutdown) {
                Err(error) => error,
                Ok(_) => panic!("shutdown unexpectedly acquired the callback lease"),
            }
        });
        assert_eq!(error.error().kind(), ErrorKind::CallbackInUse);
        assert!(error.state().is_some());
        let (session, _) = error.into_parts();

        let pending = block_on(
            session
                .expect("callback conflict retains the session")
                .shutdown(),
        )
        .unwrap();
        let error = crate::sys::with_callback_lease_for_test(|| {
            let (restart, _progress) = pending.restart_with_progress();
            match block_on(restart) {
                Err(error) => error,
                Ok(_) => panic!("restart unexpectedly acquired the callback lease"),
            }
        });
        assert_eq!(error.error().kind(), ErrorKind::CallbackInUse);
        assert!(error.state().is_some());
        let (pending, _) = error.into_parts();
        let completion = block_on(
            pending
                .expect("callback conflict retains the pending state")
                .leave_stopped(),
        )
        .unwrap();
        block_on(completion.end()).unwrap();
    }

    #[test]
    fn wrong_command_state_is_reported_without_changing_the_worker_state() {
        let primary = block_on(RestartSession::new()).unwrap();
        let (reply, receiver) = oneshot::channel();
        primary
            .worker
            .send(Command::JoinedRegister {
                resources: ResourceBatch::new(),
                reply,
            })
            .unwrap();
        let error = block_on(receive(receiver)).unwrap().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::OperationOutOfSequence);
        block_on(primary.end()).unwrap();
    }

    #[test]
    fn internal_shutdown_state_mismatch_does_not_expose_retryable_state() {
        let session = block_on(RestartSession::new()).unwrap();
        let key = session.session_key().clone();
        let cancellation = session.cancellation_handle();
        let pending = block_on(session.shutdown()).unwrap();
        let RestartPending {
            worker,
            shutdown: _,
        } = pending;
        let (reply, receiver) = oneshot::channel();
        worker
            .send(Command::Shutdown {
                options: ShutdownOptions::default(),
                reply,
            })
            .unwrap();
        let error = match block_on(ShutdownFuture {
            worker: Some(worker),
            key: Some(key),
            cancellation: Some(cancellation),
            receiver,
            cancel_on_drop: true,
        }) {
            Err(error) => error,
            Ok(_) => panic!("shutdown unexpectedly accepted the pending worker state"),
        };
        assert_eq!(error.error().kind(), ErrorKind::OperationOutOfSequence);
        assert!(error.state().is_none());
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
        let exit = pending.worker.exit_notification();
        drop(pending.restart());
        exit.wait();

        let session = block_on(RestartSession::new()).unwrap();
        let pending = block_on(session.shutdown()).unwrap();
        let exit = pending.worker.exit_notification();
        let (restart, _progress) = pending.restart_with_progress();
        drop(restart);
        exit.wait();

        crate::RestartSession::new().unwrap().end().unwrap();
    }

    #[test]
    fn dropping_shutdown_future_leaves_cleanup_with_the_worker() {
        let session = block_on(RestartSession::new()).unwrap();
        let exit = session.worker.exit_notification();
        drop(session.shutdown());
        exit.wait();
        crate::RestartSession::new().unwrap().end().unwrap();
    }
}
