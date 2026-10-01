//! `Session::close` delivers what the session queued before closing, within a deadline,
//! while `abort` still closes at once.
//!
//! Deterministic: simulated time over the mock transport.

mod support;

use std::time::Duration;

use moq_net::{Error, Hop, Timestamp, Version};
use support::harness::{MockConnectOptions, MockPair, connect_mock};

const TIMEOUT: Duration = Duration::from_secs(10);

fn produce_origin(hop: u64) -> moq_net::origin::Producer {
	let (producer, driver) = moq_net::origin::Producer::new(moq_net::origin::Config::new(Hop::new(hop).unwrap()));
	support::harness::spawn(driver);
	producer
}

/// The payloads the server read, then how the track ended.
type Read = (Vec<Vec<u8>>, Result<(), Error>);

/// A client publishing one track to a server that subscribes to it.
struct Setup {
	pair: MockPair,
	track: moq_net::track::Producer,
	reader: moq_net_sim::JoinHandle<Read>,
	_broadcast: moq_net::broadcast::Producer,
}

async fn setup() -> Setup {
	let publisher = produce_origin(1);
	let broadcast = publisher.create_broadcast("bcast").unwrap();
	let track = broadcast.create_track("video", None).unwrap();
	broadcast.announce(Default::default()).unwrap();

	let subscriber = produce_origin(2);
	let mut options = MockConnectOptions::new("moq-lite-05".parse::<Version>().unwrap());
	options.client_publish = Some(publisher.consume());
	options.server_subscribe = Some(subscriber.clone());
	let pair = connect_mock(options).await;

	let consumer = subscriber.consume();
	moq_net_sim::timeout(TIMEOUT, consumer.routed("bcast"))
		.await
		.expect("announce timeout")
		.expect("routed");
	let remote = moq_net_sim::timeout(TIMEOUT, consumer.request_broadcast("bcast"))
		.await
		.expect("resolve timeout")
		.expect("broadcast resolves");

	// The publisher only learns of a subscription once the subscriber polls it.
	let reader = moq_net_sim::spawn(async move {
		let mut sub = remote.track("video").unwrap().subscribe(None).await.expect("subscribe");
		let mut got = Vec::new();
		loop {
			let mut group = match sub.recv_group().await {
				Ok(Some(group)) => group,
				Ok(None) => return (got, Ok(())),
				Err(err) => return (got, Err(err)),
			};
			loop {
				match group.read_frame().await {
					Ok(Some(frame)) => got.push(frame.payload.to_vec()),
					Ok(None) => break,
					Err(err) => return (got, Err(err)),
				}
			}
		}
	});

	moq_net_sim::timeout(TIMEOUT, track.demand().used())
		.await
		.expect("no subscriber appeared")
		.unwrap();

	Setup {
		pair,
		track,
		reader,
		_broadcast: broadcast,
	}
}

/// A finished track's last group and FIN reach the subscriber, even though the
/// close is requested before the session wrote them.
#[moq_net_sim::test]
async fn close_delivers_a_finished_track() {
	let Setup {
		pair, track, reader, ..
	} = setup().await;

	let mut group = track.append_group().unwrap();
	group.write_frame(Timestamp::ZERO, b"last".as_slice()).unwrap();
	group.finish().unwrap();
	track.finish().unwrap();

	let started = moq_net_sim::now();
	moq_net_sim::timeout(TIMEOUT, pair.client.close())
		.await
		.expect("close timed out")
		.expect("the close drains");
	assert!(
		(moq_net_sim::now() - started) < Duration::from_secs(1),
		"drained before the deadline"
	);

	let (got, end) = moq_net_sim::timeout(TIMEOUT, reader)
		.await
		.expect("reader timed out")
		.unwrap();
	assert_eq!(got, vec![b"last".to_vec()]);
	end.expect("the track finishes");
}

/// A track that never finishes holds the drain until the deadline, then the
/// session closes anyway.
#[moq_net_sim::test]
async fn close_gives_up_on_a_live_track() {
	let Setup {
		pair, track: _track, ..
	} = setup().await;

	let started = moq_net_sim::now();
	let res = moq_net_sim::timeout(TIMEOUT, pair.client.close())
		.await
		.expect("close timed out");
	assert!(matches!(res, Err(Error::Timeout)), "{res:?}");
	assert_eq!((moq_net_sim::now() - started), Duration::from_secs(1));
}

/// An abort from another handle cuts a drain short.
#[moq_net_sim::test]
async fn abort_cuts_a_drain_short() {
	let Setup {
		pair, track: _track, ..
	} = setup().await;

	let other = pair.client.clone();
	let started = moq_net_sim::now();
	let close = moq_net_sim::spawn(pair.client.close());
	moq_net_sim::sleep(Duration::from_millis(100)).await;
	other.abort(Error::Cancel);

	let res = moq_net_sim::timeout(TIMEOUT, close)
		.await
		.expect("close timed out")
		.unwrap();
	assert!(res.is_err() && !matches!(res, Err(Error::Timeout)), "{res:?}");
	assert!(
		(moq_net_sim::now() - started) < Duration::from_secs(1),
		"{res:?} after {:?}",
		(moq_net_sim::now() - started)
	);
}
