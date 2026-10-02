//! Edge/core admission and split horizon over encrypted qmux links.

use std::time::Duration;

use moq_net::{origin, stats};
use moq_relay::{
	Config, Relay,
	cluster::{Peer, Role},
};

const TIMEOUT: Duration = Duration::from_secs(15);

struct Node {
	url: url::Url,
	origin: origin::Producer,
	stats: stats::Registry,
	trigger: moq_relay::shutdown::Trigger,
	run: tokio::task::JoinHandle<anyhow::Result<()>>,
	auth: tokio::task::JoinHandle<()>,
}

impl Node {
	async fn start(id: u64, role: Role, peers: &[&Node]) -> Self {
		let mut config = Config::default();
		config.listen.tcp.bind = Some("127.0.0.1:0".parse().unwrap());
		config.listen.tcp.tls = Some(true);
		config.listen.tls.generate = vec!["localhost".into()];
		config.connect.tls.insecure = Some(true);
		config.cluster.id = Some(id);
		config.cluster.role = Some(role);
		config.cluster.connect = peers
			.iter()
			.map(|peer| Peer::new(peer.url.as_str()).with_role(Role::Core).with_token("cluster"))
			.collect();
		config.stats.enabled = true;
		config.stats.node = Some(id.to_string());
		config.drain_timeout = Duration::ZERO;
		let mut relay = Relay::load(config).await.unwrap();
		let addr = relay.tcp_addr().unwrap();
		let url = format!("tls://localhost:{}/", addr.port()).parse().unwrap();
		let origin = relay.cluster().origin.clone();
		let stats = relay.cluster().stats.clone();
		let trigger = relay.shutdown_trigger().clone();
		let ready = relay.ready();
		let mut admissions = relay.admissions().unwrap();
		let auth = tokio::spawn(async move {
			while let Some(admission) = admissions.next().await {
				let patterns = [moq_auth::Pattern::all()].into_iter().collect::<moq_auth::Patterns>();
				let mut grant = moq_auth::Grant::new(patterns.clone(), patterns);
				grant.peer = admission.request.query.as_deref().is_some_and(|query| {
					url::form_urlencoded::parse(query.as_bytes()).any(|(key, value)| key == "jwt" && value == "cluster")
				});
				admission.grant(moq_auth::lease::Consumer::fixed(grant));
			}
		});
		let run = tokio::spawn(relay.run());
		ready.wait().await.unwrap();
		Self {
			url,
			origin,
			stats,
			trigger,
			run,
			auth,
		}
	}

	async fn stop(self) {
		self.trigger.start();
		self.run.await.unwrap().unwrap();
		self.auth.abort();
	}
}

fn client() -> moq_tokio::Client {
	let mut config = moq_tokio::connect::Config::default();
	config.tls.insecure = Some(true);
	config.once = Some(true);
	config.init(Default::default()).unwrap().with_reconnect(false)
}

async fn subscriber(node: &Node) -> (moq_tokio::Connection, origin::Consumer) {
	let origin = moq_tokio::origin::spawn();
	let consumer = origin.consume();
	let connection = client().with_subscriber(origin).connect(node.url.clone());
	let connection = connection.established().await.unwrap();
	(connection, consumer)
}

// The native multi-transport client makes these futures large, as in hidden_cluster.
fn run<F: Future<Output = ()> + Send + 'static>(test: F) {
	std::thread::Builder::new()
		.stack_size(32 * 1024 * 1024)
		.spawn(move || {
			let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
			tokio::runtime::Builder::new_current_thread()
				.enable_all()
				.build()
				.unwrap()
				.block_on(async {
					tokio::time::timeout(TIMEOUT, test)
						.await
						.expect("cluster role test timed out");
				});
		})
		.unwrap()
		.join()
		.unwrap();
}

#[test]
fn core_refuses_end_users() {
	run(async {
		let core = Node::start(1, Role::Core, &[]).await;
		let connection = client().connect(core.url.clone());
		assert!(connection.closed().await.is_err());
		core.stop().await;
	});
}

#[test]
fn edge_never_connects_cores_through_itself() {
	run(async {
		let first = Node::start(1, Role::Core, &[]).await;
		let second = Node::start(2, Role::Core, &[]).await;
		let edge = Node::start(3, Role::Edge, &[&first, &second]).await;
		let broadcast = first.origin.create_broadcast("private").unwrap();
		broadcast.announce(Default::default()).unwrap();
		let marker = second.origin.create_broadcast("second-ready").unwrap();
		marker.announce(Default::default()).unwrap();
		let consumer = edge.origin.consume();
		consumer.routed("private").await.unwrap();
		consumer.routed("second-ready").await.unwrap();
		// Both core links have delivered a route. On real sockets a negative
		// assertion needs a bounded observation window rather than mocked time.
		assert!(
			tokio::time::timeout(Duration::from_secs(1), second.origin.consume().routed("private"))
				.await
				.is_err()
		);
		drop(broadcast);
		drop(marker);
		edge.stop().await;
		second.stop().await;
		first.stop().await;
	});
}

#[test]
fn two_edges_share_one_cross_region_subscription() {
	run(async {
		let far = Node::start(1, Role::Core, &[]).await;
		let first = Node::start(2, Role::Core, &[&far]).await;
		let second = Node::start(3, Role::Core, &[&far]).await;
		let edge_a = Node::start(4, Role::Edge, &[&first, &second]).await;
		let edge_b = Node::start(5, Role::Edge, &[&first, &second]).await;
		let broadcast = far.origin.create_broadcast("shared").unwrap();
		broadcast.announce(Default::default()).unwrap();
		let track = broadcast.create_track("video", None).unwrap();
		first.origin.consume().routed("shared").await.unwrap();
		second.origin.consume().routed("shared").await.unwrap();
		let (_a, a) = subscriber(&edge_a).await;
		let (_b, b) = subscriber(&edge_b).await;
		let a = a.routed_broadcast("shared").await.unwrap();
		let b = b.routed_broadcast("shared").await.unwrap();
		let mut a = a.track("video").unwrap().subscribe(None).await.unwrap();
		let mut b = b.track("video").unwrap().subscribe(None).await.unwrap();
		let mut group = track.append_group().unwrap();
		group
			.write_frame(moq_net::Timestamp::ZERO, b"payload".as_ref())
			.unwrap();
		group.finish().unwrap();
		a.recv_group()
			.await
			.unwrap()
			.unwrap()
			.read_frame()
			.await
			.unwrap()
			.unwrap();
		b.recv_group()
			.await
			.unwrap()
			.unwrap()
			.read_frame()
			.await
			.unwrap()
			.unwrap();
		let mut report = stats::Report::default();
		far.stats.report(&mut report);
		let shared = report
			.traffic
			.iter()
			.find(|entry| entry.path.as_str() == "shared")
			.unwrap();
		assert_eq!(
			shared.publisher.subscriptions_started, 1,
			"both edges must choose the same regional core"
		);
		drop(a);
		drop(b);
		edge_b.stop().await;
		edge_a.stop().await;
		second.stop().await;
		first.stop().await;
		far.stop().await;
	});
}
