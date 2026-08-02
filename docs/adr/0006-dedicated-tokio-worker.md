# ADR 0006: Dedicated Tokio worker

Status: accepted for the initial 0.1.0 release.

Moving a blocking session in and out of unrelated `spawn_blocking` jobs can
lose the owner when an awaiting future is cancelled. Each async session instead
owns one standard worker thread. The blocking typestate never leaves that
thread; commands arrive through one FIFO channel.

Tokio oneshot channels carry operation results. Watch channels provide
cloneable progress receivers and intentionally coalesce samples. User code does
not execute on the worker. Progress closure is normal stream completion;
worker failure remains an error on the paired operation future.

Dropping a shutdown future requests cancellation and disconnects its handle.
The worker then drops its pending state, which performs recovery and end.
Dropping a restart future only disconnects the waiter, so the queued restart
finishes before state cleanup.

The facade is optional. There is no runtime-neutral executor trait, backend
trait, async-trait dependency, or generic dependency-injection surface.
