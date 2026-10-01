//! What the peer declared in its SETUP, and the slot the session tasks read it from.

/// The Setup Options the peer sent us.
///
/// One value per session: extensions ride the same SETUP, so they arrive together
/// and are read together.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Peer {
	/// MoQ Solicit: whether the peer requires advertisements to be solicited, so an
	/// unsolicited PUBLISH_NAMESPACE is unwanted. `None` when it declared nothing, which
	/// is the one case where sending us one anyway is not a protocol violation.
	pub solicit: Option<bool>,

	/// MoQ Hidden: whether the peer understands the HIDDEN parameter on
	/// SUBSCRIBE_NAMESPACE, so we may send it.
	pub hidden: bool,
}

/// Shared slot for [`Peer`], filled when the peer's SETUP is read.
///
/// The publisher blocks on this before its first advertisement: the SETUP decides both
/// whether an advertisement may be sent unasked (MoQ Solicit) and which hidden
/// namespaces it may receive, so nothing can go out until it arrives. Every handle
/// shares the same slot.
#[derive(Clone, Default)]
pub(crate) struct PeerSetup(kio::Shared<Option<Peer>>);

impl PeerSetup {
	/// Record what the peer declared. A SETUP carrying no options records the default
	/// (no extension negotiated, no requirements), which is what unblocks a waiter.
	///
	/// First write wins. The announce loops read this once and hold it for their
	/// lifetime while subscription serving re-reads it, so letting a later SETUP
	/// overwrite the options would change how existing request streams are handled.
	pub fn set(&self, peer: Peer) {
		let mut slot = self.0.lock();
		if slot.is_none() {
			*slot = Some(peer);
		}
	}

	/// Await the peer's SETUP.
	///
	/// The peer MUST send exactly one, so this resolves once that stream is read. Waits
	/// forever if it never does; the caller is a session task, cancelled when the driver
	/// drops.
	pub async fn get(&self) -> Peer {
		let slot = self
			.0
			.wait(|peer| match peer.is_some() {
				true => std::task::Poll::Ready(()),
				false => std::task::Poll::Pending,
			})
			.await;
		(*slot).expect("waited for Some")
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[tokio::test]
	async fn first_write_wins() {
		let first = Peer {
			solicit: None,
			hidden: false,
		};
		let slot = PeerSetup::default();
		slot.set(first);
		slot.set(Peer {
			solicit: Some(true),
			hidden: true,
		});
		assert_eq!(slot.get().await, first);
	}
}
