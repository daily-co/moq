//! A single-threaded, deterministic executor with simulated time, for moq-net's tests.
//!
//! Time never moves on its own. Once every task has stalled, [`run`] jumps the clock
//! to the earliest armed timer and wakes it, like tokio's paused clock, so a test
//! never waits on the wall clock. A run whose tasks are all parked with nothing
//! armed can never finish, so it panics rather than hang.
//!
//! Protocol drivers take time from their caller: [`drive`] polls one with the
//! simulated instant, and [`attach`] keeps an externally owned clock in step.

use std::{
	cell::RefCell,
	collections::BTreeMap,
	future::{Future, IntoFuture},
	pin::{Pin, pin},
	rc::Rc,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	task::{Context, Poll, Wake, Waker},
	time::{Duration, Instant},
};

use futures::{
	FutureExt,
	executor::{LocalPool, LocalSpawner},
	task::LocalSpawnExt,
};

pub use moq_net_sim_macros::test;

thread_local! {
	static CURRENT: RefCell<Option<Rc<Runtime>>> = const { RefCell::new(None) };
}

/// A source of deadlines kept in step with simulated time: called with the new
/// instant whenever time moves, it returns its earliest deadline still pending.
type Source = Box<dyn FnMut(Instant) -> Option<Instant>>;

struct Runtime {
	spawner: LocalSpawner,
	time: RefCell<Time>,
	sources: RefCell<Vec<Source>>,
}

struct Time {
	now: Instant,
	next: u64,
	timers: BTreeMap<(Instant, u64), Waker>,
}

impl Runtime {
	fn now(&self) -> Instant {
		self.time.borrow().now
	}

	/// Move time to `at` (never backwards), waking every timer and source due by then.
	fn advance_to(&self, at: Instant) {
		let due = {
			let mut time = self.time.borrow_mut();
			time.now = time.now.max(at);
			let now = time.now;
			let mut due = Vec::new();
			while let Some(entry) = time.timers.first_entry() {
				if entry.key().0 > now {
					break;
				}
				due.push(entry.remove());
			}
			due
		};
		due.into_iter().for_each(Waker::wake);
		self.poll_sources();
	}

	/// Bring every source up to the current instant, returning the earliest deadline they report.
	fn poll_sources(&self) -> Option<Instant> {
		let now = self.now();
		// Taken out so a source may attach another while it runs.
		let mut sources = std::mem::take(&mut *self.sources.borrow_mut());
		let next = sources.iter_mut().filter_map(|source| source(now)).min();
		let mut current = self.sources.borrow_mut();
		sources.append(&mut current);
		*current = sources;
		next
	}

	/// Jump to the earliest armed timer or source deadline. False when nothing is armed.
	fn advance(&self) -> bool {
		let timer = self.time.borrow().timers.first_key_value().map(|((at, _), _)| *at);
		let source = self.poll_sources();
		match timer.into_iter().chain(source).min() {
			Some(at) => {
				self.advance_to(at);
				true
			}
			None => false,
		}
	}
}

fn current() -> Rc<Runtime> {
	CURRENT
		.with(|current| current.borrow().clone())
		.expect("not inside moq_net_sim::run")
}

/// Whether the calling thread is inside [`run`].
pub fn is_running() -> bool {
	CURRENT.with(|current| current.borrow().is_some())
}

/// Wakes the root future by raising a flag the run loop checks.
struct Flag(AtomicBool);

impl Wake for Flag {
	fn wake(self: Arc<Self>) {
		self.0.store(true, Ordering::SeqCst);
	}
}

/// Clears the thread's runtime when [`run`] returns or unwinds.
struct Enter;

impl Drop for Enter {
	fn drop(&mut self) {
		CURRENT.with(|current| current.borrow_mut().take());
	}
}

/// Run `future` to completion, along with everything it spawns, in simulated time.
///
/// Spawned tasks still pending when `future` finishes are dropped.
///
/// # Panics
///
/// When called inside another `run`, and when every task is parked with no timer
/// armed, which is a deadlock.
pub fn run<F: IntoFuture>(future: F) -> F::Output {
	let mut pool = LocalPool::new();
	let runtime = Rc::new(Runtime {
		spawner: pool.spawner(),
		time: RefCell::new(Time {
			now: Instant::now(),
			next: 0,
			timers: BTreeMap::new(),
		}),
		sources: RefCell::new(Vec::new()),
	});
	CURRENT.with(|current| {
		let mut current = current.borrow_mut();
		assert!(current.is_none(), "moq_net_sim::run cannot nest");
		*current = Some(runtime.clone());
	});
	let enter = Enter;

	let mut future = pin!(future.into_future());
	let flag = Arc::new(Flag(AtomicBool::new(true)));
	let waker = Waker::from(flag.clone());
	let mut cx = Context::from_waker(&waker);
	let output = loop {
		if flag.0.swap(false, Ordering::SeqCst)
			&& let Poll::Ready(output) = future.as_mut().poll(&mut cx)
		{
			break output;
		}
		pool.run_until_stalled();
		if flag.0.load(Ordering::SeqCst) {
			continue;
		}
		assert!(
			runtime.advance(),
			"deadlock: every task is parked and no timer is armed"
		);
	};

	drop(pool);
	drop(enter);
	// Sources and timers may hold wakers into the dropped tasks.
	runtime.sources.borrow_mut().clear();
	runtime.time.borrow_mut().timers.clear();
	output
}

/// The current simulated instant.
///
/// It starts at the real instant [`run`] was entered and advances only when every
/// task has stalled, or by [`advance`].
pub fn now() -> Instant {
	current().now()
}

/// Keep an externally owned clock in step with simulated time.
///
/// `source` is called with the new instant whenever time moves and returns its
/// earliest pending deadline, which [`run`] treats like an armed timer.
pub fn attach(source: impl FnMut(Instant) -> Option<Instant> + 'static) {
	current().sources.borrow_mut().push(Box::new(source));
}

/// Advance simulated time by `duration`, waking everything due, then yield.
pub async fn advance(duration: Duration) {
	let runtime = current();
	let at = runtime.now() + duration;
	runtime.advance_to(at);
	yield_now().await;
}

/// Yield once, letting every other ready task run first.
pub fn yield_now() -> impl Future<Output = ()> {
	let mut yielded = false;
	std::future::poll_fn(move |cx| {
		if yielded {
			return Poll::Ready(());
		}
		yielded = true;
		cx.waker().wake_by_ref();
		Poll::Pending
	})
}

/// A future that completes at a simulated instant.
pub struct Sleep {
	runtime: Rc<Runtime>,
	at: Instant,
	key: Option<(Instant, u64)>,
}

/// Complete once simulated time reaches `at`.
pub fn sleep_until(at: Instant) -> Sleep {
	Sleep {
		runtime: current(),
		at,
		key: None,
	}
}

/// Complete once `duration` of simulated time has passed.
pub fn sleep(duration: Duration) -> Sleep {
	sleep_until(now() + duration)
}

impl Sleep {
	/// The instant this completes at.
	pub fn deadline(&self) -> Instant {
		self.at
	}

	/// Complete at `at` instead.
	pub fn reset(&mut self, at: Instant) {
		self.disarm();
		self.at = at;
	}

	fn disarm(&mut self) {
		if let Some(key) = self.key.take() {
			self.runtime.time.borrow_mut().timers.remove(&key);
		}
	}
}

impl Future for Sleep {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
		let this = &mut *self;
		let mut time = this.runtime.time.borrow_mut();
		if time.now >= this.at {
			if let Some(key) = this.key.take() {
				time.timers.remove(&key);
			}
			return Poll::Ready(());
		}
		let key = *this.key.get_or_insert_with(|| {
			let id = time.next;
			time.next += 1;
			(this.at, id)
		});
		time.timers.insert(key, cx.waker().clone());
		Poll::Pending
	}
}

impl Drop for Sleep {
	fn drop(&mut self) {
		self.disarm();
	}
}

/// [`timeout`] ran out before its future completed.
#[derive(Debug, PartialEq, Eq)]
pub struct Elapsed;

impl std::fmt::Display for Elapsed {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("deadline elapsed")
	}
}

impl std::error::Error for Elapsed {}

/// Run `future` for at most `duration` of simulated time, counted from this call.
pub fn timeout<F: IntoFuture>(duration: Duration, future: F) -> impl Future<Output = Result<F::Output, Elapsed>> {
	let mut sleep = sleep(duration);
	let mut future = Box::pin(future.into_future());
	std::future::poll_fn(move |cx| {
		if let Poll::Ready(output) = future.as_mut().poll(cx) {
			return Poll::Ready(Ok(output));
		}
		Pin::new(&mut sleep).poll(cx).map(|()| Err(Elapsed))
	})
}

/// Run a caller-driven state machine until it returns an error.
///
/// `poll` is called with the simulated instant and returns when it next wants to
/// be polled: `Ok(Some(at))` by `at` or on a wake, `Ok(None)` only on a wake.
pub async fn drive<E>(mut poll: impl FnMut(Instant, &kio::Waiter) -> Result<Option<Instant>, E> + Unpin) -> E {
	let runtime = current();
	let mut timer: Option<Sleep> = None;
	kio::wait(move |waiter| {
		loop {
			let at = match poll(runtime.now(), waiter) {
				Ok(Some(at)) => at,
				Ok(None) => {
					timer = None;
					return Poll::Pending;
				}
				Err(err) => return Poll::Ready(err),
			};
			let sleep = timer.get_or_insert_with(|| sleep_until(at));
			if sleep.deadline() != at {
				sleep.reset(at);
			}
			if waiter.poll_future(Pin::new(sleep)).is_pending() {
				return Poll::Pending;
			}
		}
	})
	.await
}

/// A spawned task failed to complete: it panicked or was aborted.
pub struct JoinError(Option<Box<dyn std::any::Any + Send>>);

impl JoinError {
	/// Whether [`JoinHandle::abort`] cancelled the task.
	pub fn is_cancelled(&self) -> bool {
		self.0.is_none()
	}

	/// Whether the task panicked.
	pub fn is_panic(&self) -> bool {
		self.0.is_some()
	}
}

impl std::fmt::Debug for JoinError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		std::fmt::Display::fmt(self, f)
	}
}

impl std::fmt::Display for JoinError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self.0 {
			None => f.write_str("task was cancelled"),
			Some(panic) => match (panic.downcast_ref::<&str>(), panic.downcast_ref::<String>()) {
				(Some(message), _) => write!(f, "task panicked: {message}"),
				(_, Some(message)) => write!(f, "task panicked: {message}"),
				_ => f.write_str("task panicked"),
			},
		}
	}
}

impl std::error::Error for JoinError {}

struct Join<T> {
	output: Option<Result<T, JoinError>>,
	finished: bool,
	aborted: bool,
	detached: bool,
	task: Option<Waker>,
	joiner: Option<Waker>,
}

/// Await a spawned task's output. Dropping it detaches the task.
pub struct JoinHandle<T> {
	join: Rc<RefCell<Join<T>>>,
}

impl<T> JoinHandle<T> {
	/// Cancel the task: it is dropped before its next poll.
	pub fn abort(&self) {
		let mut join = self.join.borrow_mut();
		join.aborted = true;
		if let Some(task) = join.task.take() {
			task.wake();
		}
	}

	/// Whether the task has completed, panicked, or been cancelled.
	pub fn is_finished(&self) -> bool {
		self.join.borrow().finished
	}
}

impl<T> Future for JoinHandle<T> {
	type Output = Result<T, JoinError>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let mut join = self.join.borrow_mut();
		if let Some(output) = join.output.take() {
			return Poll::Ready(output);
		}
		assert!(!join.finished, "JoinHandle polled after completion");
		join.joiner = Some(cx.waker().clone());
		Poll::Pending
	}
}

impl<T> Drop for JoinHandle<T> {
	fn drop(&mut self) {
		self.join.borrow_mut().detached = true;
	}
}

/// Spawn a task on the running executor.
///
/// A panic is kept for the [`JoinHandle`], or propagated out of [`run`] once the
/// handle is dropped, so a detached task cannot fail silently.
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
	F: Future + 'static,
	F::Output: 'static,
{
	let join = Rc::new(RefCell::new(Join {
		output: None,
		finished: false,
		aborted: false,
		detached: false,
		task: None,
		joiner: None,
	}));
	let state = join.clone();
	let mut future = Some(Box::pin(future));
	let task = std::future::poll_fn(move |cx| {
		let finish = |output| {
			let mut join = state.borrow_mut();
			join.finished = true;
			if join.detached {
				if let Err(JoinError(Some(panic))) = output {
					std::panic::resume_unwind(panic);
				}
				return;
			}
			join.output = Some(output);
			if let Some(joiner) = join.joiner.take() {
				joiner.wake();
			}
		};

		if state.borrow().aborted {
			future = None;
			finish(Err(JoinError(None)));
			return Poll::Ready(());
		}
		state.borrow_mut().task = Some(cx.waker().clone());

		let running = future.as_mut().expect("task polled after completion");
		let output = match std::panic::AssertUnwindSafe(running.as_mut())
			.catch_unwind()
			.poll_unpin(cx)
		{
			Poll::Pending => return Poll::Pending,
			Poll::Ready(output) => output,
		};
		future = None;
		finish(output.map_err(|panic| JoinError(Some(panic))));
		Poll::Ready(())
	});
	current().spawner.spawn_local(task).expect("the executor has shut down");
	JoinHandle { join }
}

#[cfg(test)]
mod tests {
	use super::{
		Duration, Elapsed, Poll, Rc, RefCell, Waker, advance, attach, drive, now, run, sleep, spawn, timeout, yield_now,
	};

	#[test]
	fn time_moves_only_when_every_task_stalls() {
		run(async {
			let start = now();
			let a = spawn(async {
				sleep(Duration::from_secs(5)).await;
				now()
			});
			let b = spawn(async {
				sleep(Duration::from_secs(2)).await;
				now()
			});
			assert_eq!(now(), start, "spawning does not move time");
			assert_eq!(b.await.unwrap(), start + Duration::from_secs(2));
			assert_eq!(a.await.unwrap(), start + Duration::from_secs(5));
		});
	}

	#[test]
	fn timeout_uses_simulated_time() {
		run(async {
			let start = now();
			let slow = timeout(Duration::from_secs(1), sleep(Duration::from_secs(60))).await;
			assert_eq!(slow, Err(Elapsed));
			assert_eq!(now(), start + Duration::from_secs(1));
			let fast = timeout(Duration::from_secs(60), async { 7 }).await;
			assert_eq!(fast, Ok(7));
		});
	}

	#[test]
	fn advance_wakes_due_timers() {
		run(async {
			let start = now();
			let timer = spawn(sleep(Duration::from_secs(3)));
			yield_now().await;
			advance(Duration::from_secs(3)).await;
			assert!(timer.is_finished());
			assert_eq!(now(), start + Duration::from_secs(3));
		});
	}

	#[test]
	fn abort_cancels_a_task() {
		run(async {
			let task = spawn(std::future::pending::<()>());
			task.abort();
			assert!(task.await.unwrap_err().is_cancelled());
		});
	}

	#[test]
	fn a_panic_waits_in_the_handle() {
		run(async {
			let task = spawn(async { panic!("boom") });
			yield_now().await;
			assert!(task.is_finished());
			assert!(task.await.unwrap_err().is_panic());
		});
	}

	#[test]
	#[should_panic(expected = "boom")]
	fn a_detached_panic_fails_the_run() {
		run(async {
			drop(spawn(async { panic!("boom") }));
			yield_now().await;
		});
	}

	#[test]
	#[should_panic(expected = "deadlock")]
	fn a_deadlock_panics() {
		run(std::future::pending::<()>());
	}

	#[test]
	fn attached_sources_advance_with_time() {
		run(async {
			let deadline = now() + Duration::from_secs(4);
			let parked: Rc<RefCell<Option<Waker>>> = Default::default();
			let wake = parked.clone();
			attach(move |now| {
				if now < deadline {
					return Some(deadline);
				}
				if let Some(waker) = wake.borrow_mut().take() {
					waker.wake();
				}
				None
			});
			// Nothing else is armed, so the stall jumps straight to the source's deadline.
			std::future::poll_fn(|cx| {
				if now() >= deadline {
					return Poll::Ready(());
				}
				*parked.borrow_mut() = Some(cx.waker().clone());
				Poll::Pending
			})
			.await;
			assert_eq!(now(), deadline);
		});
	}

	#[test]
	fn drive_polls_at_each_requested_instant() {
		run(async {
			let start = now();
			let mut polls = Vec::new();
			let err = drive(|now, _waiter| {
				polls.push(now - start);
				match polls.len() {
					3 => Err("done"),
					n => Ok(Some(now + Duration::from_secs(n as u64))),
				}
			})
			.await;
			assert_eq!(err, "done");
			assert_eq!(polls, [Duration::ZERO, Duration::from_secs(1), Duration::from_secs(3)]);
		});
	}
}
