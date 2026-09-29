//! One track's timeline, followed across transient subscription failures.
//!
//! A transport error on a timeline subscription (a reset stream, a lost session, the publisher's
//! track aborting) subscribes again rather than ending the timeline. The fresh subscription opens
//! on a group header that restates the retained window, so its events are reconciled against the
//! ones already delivered: rows the source popped during the outage are popped, retained rows the
//! header's checkpoint omitted are kept, and restated records are not delivered twice.
//!
//! Nothing waits on a timer. A re-subscribe is either served, refused (the track is no longer
//! published, or the broadcast went away), or fails before delivering anything; the last two end
//! the timeline with an error. A malformed timeline fails the same way on the first try, since a
//! fresh subscription would only decode the same retained record again.

use std::collections::VecDeque;
use std::ops::Range;

use hang::catalog::Archive;
use moq_mux::timeline::{Consumer, Entry, Event};

/// Reads one track's timeline, re-subscribing after a transport error.
pub(crate) struct Follower {
	broadcast: moq_net::broadcast::Consumer,
	section: Archive,
	track: String,
	consumer: Consumer,
	/// Whether `consumer` has not yielded an event yet.
	fresh: bool,
	/// The window indices delivered and not since popped or skipped, once anything was.
	held: Option<Range<u64>>,
	/// The newest delivered record, to tell a restated timeline from a restarted one.
	last: Option<Entry>,
	/// Events reconciliation produced ahead of the one being handled.
	queued: VecDeque<Event>,
}

impl Follower {
	/// Subscribe to `track`'s timeline as advertised by `section`.
	pub async fn subscribe(
		broadcast: &moq_net::broadcast::Consumer,
		section: &Archive,
		track: &str,
	) -> moq_mux::Result<Self> {
		Ok(Self {
			consumer: Consumer::subscribe(broadcast, section, track).await?,
			broadcast: broadcast.clone(),
			section: section.clone(),
			track: track.to_string(),
			fresh: true,
			held: None,
			last: None,
			queued: VecDeque::new(),
		})
	}

	/// The next window event, or `None` once the timeline ends cleanly.
	pub async fn next(&mut self) -> moq_mux::Result<Option<Event>> {
		loop {
			if let Some(event) = self.queued.pop_front() {
				return Ok(Some(event));
			}
			match self.consumer.next().await {
				Ok(Some(event)) => self.reconcile(event),
				Ok(None) => return Ok(None),
				// A subscription that failed before delivering anything is not retried: it would
				// fail the same way again, as fast as the source can refuse it.
				Err(err) if is_transport(&err) && !self.fresh => {
					tracing::warn!(track = %self.track, %err, "timeline subscription failed; subscribing again");
					self.consumer = Consumer::subscribe(&self.broadcast, &self.section, &self.track).await?;
					self.fresh = true;
				}
				Err(err) => return Err(err),
			}
		}
	}

	/// Queue what `event` means to a reader holding [`held`](Self::held).
	fn reconcile(&mut self, event: Event) {
		let fresh = std::mem::replace(&mut self.fresh, false);
		let Some(held) = self.held.clone() else {
			return self.deliver(event);
		};

		if fresh {
			// A fresh subscription's first event names the window's front: its header's offset.
			let front = match &event {
				Event::Push { index, .. } => *index,
				Event::Pop(range) | Event::Skip(range) => range.start,
				_ => unreachable!("unknown timeline event"),
			};
			// The source popped these while this reader was away.
			let popped = held.start..front.min(held.end);
			if !popped.is_empty() {
				self.deliver(Event::Pop(popped));
			}
		}

		let held = self.held.clone().expect("held survives a pop");
		match event {
			// Records the header restated: this reader holds them unless there's a hole before the
			// next one it needs.
			Event::Skip(range) if range.end <= held.end => {}
			Event::Skip(range) => self.deliver(Event::Skip(held.end..range.end)),
			Event::Push { index, entry } if index < held.end => {
				// A restated record. The newest one this reader holds must match, or the source
				// restarted its timeline under the same track: start over from there.
				if index + 1 == held.end && self.last.as_ref().is_some_and(|last| *last != entry) {
					self.deliver(Event::Skip(held));
					self.deliver(Event::Push { index, entry });
				}
			}
			Event::Push { index, entry } if index > held.end => {
				// Records pushed and popped while this reader was away.
				self.deliver(Event::Skip(held.end..index));
				self.deliver(Event::Push { index, entry });
			}
			event => self.deliver(event),
		}
	}

	/// Queue `event` for the reader and track what it now holds.
	fn deliver(&mut self, event: Event) {
		let held = self.held.get_or_insert(0..0);
		match &event {
			Event::Push { index, entry } => {
				if held.is_empty() {
					held.start = *index;
				}
				held.end = index + 1;
				self.last = Some(entry.clone());
			}
			Event::Pop(range) => held.start = held.start.max(range.end).min(held.end),
			// A reader forgets everything on a skip, since the next record can't follow its last.
			Event::Skip(range) => {
				*held = range.end..range.end;
				self.last = None;
			}
			_ => unreachable!("unknown timeline event"),
		}
		self.queued.push_back(event);
	}
}

/// Whether `err` came from the transport rather than the timeline's content.
fn is_transport(err: &moq_mux::Error) -> bool {
	matches!(
		err,
		moq_mux::Error::Moq(_) | moq_mux::Error::Json(moq_json::Error::Net(_))
	)
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use hang::timeline::{Position, Record};
	use moq_json::window::Checkpoint;

	use super::*;

	const TRACK: &str = "video0";

	fn section() -> Archive {
		let mut section = Archive::new();
		section
			.timelines
			.insert(TRACK.to_string(), hang::timeline::default_name(TRACK));
		section
	}

	fn record(sequence: u64, pts: u64) -> Record {
		Record::new(
			sequence,
			pts,
			2_000,
			Position::group(sequence),
			Position::group(sequence + 1),
		)
	}

	/// Publish the timeline track, continuing `records` when set, and keep a handle to abort it.
	fn publish(
		broadcast: &moq_net::broadcast::Producer,
		records: Option<Vec<Record>>,
	) -> (moq_net::track::Producer, moq_mux::timeline::Producer) {
		let track = broadcast
			.create_track(hang::timeline::default_name(TRACK), moq_mux::timeline::Producer::info())
			.unwrap();
		let timeline = match records {
			Some(records) => {
				let start = records.first().map_or(0, |record| record.sequence);
				let end = records.last().map_or(0, |record| record.sequence + 1);
				let checkpoint = Checkpoint {
					range: start..end,
					records,
				};
				moq_mux::timeline::Producer::resume(track.clone(), &checkpoint).unwrap()
			}
			None => moq_mux::timeline::Producer::new(track.clone()),
		};
		(track, timeline)
	}

	/// The publisher's session is lost: its track and open group end with an error.
	fn abort(track: moq_net::track::Producer, timeline: moq_mux::timeline::Producer) {
		drop(timeline);
		track.abort(moq_net::Error::SessionClosed).unwrap();
	}

	#[derive(Debug, PartialEq)]
	enum Seen {
		Push(u64),
		Pop(Range<u64>),
		Skip(Range<u64>),
	}

	fn seen(event: moq_mux::Result<Option<Event>>) -> Seen {
		match event.expect("the timeline is readable").expect("the timeline is live") {
			Event::Push { index, .. } => Seen::Push(index),
			Event::Pop(range) => Seen::Pop(range),
			Event::Skip(range) => Seen::Skip(range),
			_ => unreachable!("unknown timeline event"),
		}
	}

	async fn next(follower: &mut Follower) -> Seen {
		seen(
			tokio::time::timeout(Duration::from_secs(5), follower.next())
				.await
				.expect("an event arrives"),
		)
	}

	/// Subscribe, then read `count` records as they're pushed, so the reader keeps up.
	async fn follow(
		broadcast: &moq_net::broadcast::Producer,
		timeline: &mut moq_mux::timeline::Producer,
		count: u64,
	) -> Follower {
		let mut follower = Follower::subscribe(&broadcast.consume(), &section(), TRACK)
			.await
			.unwrap();
		for sequence in 0..count {
			timeline.push(&record(sequence, sequence * 2_000)).unwrap();
			assert_eq!(next(&mut follower).await, Seen::Push(sequence));
		}
		follower
	}

	/// Read across an outage: the re-subscribe waits on the track, not a timer, until `restore`.
	async fn across<T>(follower: &mut Follower, restore: impl FnOnce() -> T) -> (moq_mux::Result<Option<Event>>, T) {
		let mut next = std::pin::pin!(follower.next());
		let waited = tokio::time::timeout(Duration::from_secs(60), &mut next).await;
		assert!(waited.is_err(), "the re-subscribe did not wait: {waited:?}");
		let restored = restore();
		let next = tokio::time::timeout(Duration::from_secs(5), next)
			.await
			.expect("nothing parks once the source answers");
		(next, restored)
	}

	#[tokio::test(start_paused = true)]
	async fn a_reconnect_past_the_checkpoint_keeps_the_retained_records() {
		let broadcast = moq_net::broadcast::Info::new().produce();
		let _handler = broadcast.dynamic();
		let (track, mut timeline) = publish(&broadcast, None);
		// More records than a group header restates (`CHECKPOINT_RECORDS`, 256).
		let mut follower = follow(&broadcast, &mut timeline, 300).await;
		abort(track, timeline);

		// The source popped ten records during the outage. Its first header restates only the
		// newest 256 of the rest, so the fresh subscription skips 10..44, which the reader holds.
		let (first, _source) = across(&mut follower, || {
			let retained = (10..300).map(|sequence| record(sequence, sequence * 2_000)).collect();
			let (track, mut timeline) = publish(&broadcast, Some(retained));
			timeline.push(&record(300, 600_000)).unwrap();
			(track, timeline)
		})
		.await;
		assert_eq!(seen(first), Seen::Pop(0..10));
		assert_eq!(next(&mut follower).await, Seen::Push(300));
	}

	#[tokio::test(start_paused = true)]
	async fn a_restarted_timeline_starts_over() {
		let broadcast = moq_net::broadcast::Info::new().produce();
		let _handler = broadcast.dynamic();
		let (track, mut timeline) = publish(&broadcast, None);
		let mut follower = follow(&broadcast, &mut timeline, 3).await;
		abort(track, timeline);

		// A new run numbers from zero again, describing other media.
		let (first, _source) = across(&mut follower, || {
			let (track, mut timeline) = publish(&broadcast, None);
			for sequence in 0..4 {
				timeline.push(&record(sequence, 100_000 + sequence * 2_000)).unwrap();
			}
			(track, timeline)
		})
		.await;
		assert_eq!(seen(first), Seen::Skip(0..3));
		assert_eq!(next(&mut follower).await, Seen::Push(2));
		assert_eq!(next(&mut follower).await, Seen::Push(3));
	}

	#[tokio::test(start_paused = true)]
	async fn a_refused_resubscribe_ends_the_timeline() {
		// No handler, so a track that isn't published is refused rather than awaited.
		let broadcast = moq_net::broadcast::Info::new().produce();
		let (track, mut timeline) = publish(&broadcast, None);
		let mut follower = follow(&broadcast, &mut timeline, 1).await;
		abort(track, timeline);

		let err = follower.next().await.expect_err("the re-subscribe is refused");
		assert!(matches!(err, moq_mux::Error::Moq(moq_net::Error::NotFound)), "{err}");
	}

	#[tokio::test(start_paused = true)]
	async fn a_pending_resubscribe_ends_with_the_broadcast() {
		let broadcast = moq_net::broadcast::Info::new().produce();
		let _handler = broadcast.dynamic();
		let (track, mut timeline) = publish(&broadcast, None);
		let mut follower = follow(&broadcast, &mut timeline, 1).await;
		abort(track, timeline);

		let (next, ()) = across(&mut follower, || broadcast.close()).await;
		let err = next.expect_err("the broadcast is gone");
		assert!(matches!(err, moq_mux::Error::Moq(moq_net::Error::Unroutable)), "{err}");
	}

	#[tokio::test(start_paused = true)]
	async fn a_resubscribe_that_fails_before_any_event_is_not_retried() {
		let broadcast = moq_net::broadcast::Info::new().produce();
		let _handler = broadcast.dynamic();
		let (track, mut timeline) = publish(&broadcast, None);
		let mut follower = follow(&broadcast, &mut timeline, 1).await;
		abort(track, timeline);

		let mut next = std::pin::pin!(follower.next());
		assert!(tokio::time::timeout(Duration::from_secs(60), &mut next).await.is_err());
		// Serve the re-subscribe, then fail it before it delivers a single event.
		let (track, timeline) = publish(&broadcast, None);
		assert!(tokio::time::timeout(Duration::from_secs(1), &mut next).await.is_err());
		abort(track, timeline);
		tokio::time::timeout(Duration::from_secs(5), next)
			.await
			.expect("a failing source is not subscribed again")
			.expect_err("the timeline ends");
	}

	#[tokio::test(start_paused = true)]
	async fn a_malformed_timeline_is_not_retried() {
		let broadcast = moq_net::broadcast::Info::new().produce();
		let _handler = broadcast.dynamic();
		let (track, mut timeline) = publish(&broadcast, None);
		let mut follower = follow(&broadcast, &mut timeline, 1).await;

		// A group that doesn't decode: a fresh subscription would read the same one again.
		let mut group = track.append_group().unwrap();
		group.write_frame(moq_net::Timestamp::now(), &b"garbage"[..]).unwrap();
		let err = tokio::time::timeout(Duration::from_secs(5), follower.next())
			.await
			.expect("a malformed timeline fails at once")
			.expect_err("the timeline is malformed");
		assert!(!is_transport(&err), "{err}");
	}
}
