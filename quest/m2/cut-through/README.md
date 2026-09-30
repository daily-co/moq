# Cut-through group streams

## Goal

A relay forwards group stream bytes that arrive past a QUIC stream hole
instead of holding them until the retransmission fills it. Payload after a
lost packet reaches the next hop at its own output offset while the hole is
still being recovered, so a loss on one hop no longer delays everything behind
it on the next.

The line first measures whether this is worth it, then builds it only if the
measurement says so. A measured no-go deletes the build quests; the loss-delay
metric and the bench stay either way.

Non-goals: end consumers (players, `moq-cli`) keep ordered reads, since a
decoder needs whole frames. No MoQ wire change; QUIC already allows a sender to
send stream offsets in any order.

## Plan

Decided 2026-09-30:

- Measure first, and plan the build now so the verdict only decides go or
  no-go, not the design. The build quests require the [bench](/quest/m2/cut-through/bench.md).
- The gain is roughly the time the relay would spend bursting the held bytes
  downstream after the hole fills: small when the egress hop has headroom,
  larger for a big I-frame on a tight hop. The bench sweeps exactly that.
- A hole that covers a frame header blocks every later frame on that stream:
  the next header's input position and every output header's length depend on
  earlier header values (lite's timestamp delta, IETF's object id delta). A hole
  inside a payload blocks nothing, since the frame sizes locate both the input
  and the output bytes.
- The frame model gains an offset write and a range read, crate-private in
  `moq-net`, since the lite and IETF publishers and subscribers that use them
  live in the same crate. Cut-through frames keep the received chunks by
  offset instead of copying into a pre-allocated buffer, so memory tracks bytes
  received and [the allocation budget](/quest/m0/frame-alloc-budget.md) is
  unaffected.
- `web-transport-trait` gains an unordered chunk read and an offset write whose
  defaults report unsupported, and moq-net falls back to ordered I/O. Only the
  noq backends implement them; browsers, qmux, and iroh keep the defaults.
  Additive, so it lands on `main`.
- Always on wherever both of the relay's streams support it. No config knob.
- The relay's own egress is noq for every native and browser viewer, so browser
  viewers benefit too; only the relay's side needs the feature.

Public API: additive `web-transport-trait` methods; the loss-delay counter on
`moq-stats` ingress rows. Wire: none for MoQ; the counter is a `moq-stats`
field.

End to end: once [the relay quest](/quest/m2/cut-through/relay.md) lands, re-run the
bench on the same sweep and record before and after in this README before
closing the line. No new doc page: nothing user-facing changes beyond the
stats field, which its quest documents inline.

## Required

- [Loss delay](/quest/m2/cut-through/loss-delay.md) - relays report, per broadcast, the ingress bytes a loss held back by at least one RTT
- [Bench](/quest/m2/cut-through/bench.md) - a lossy relay hop swept over loss, frame size, and egress headroom, against a direct-connection bound, with a go or no-go verdict
- [Offset writes in noq](/quest/m2/cut-through/noq.md) - the fork's send streams accept writes past a gap and send them right away
- [Frame ranges](/quest/m2/cut-through/frames.md) - a frame fills out of order and a reader sees each range as it lands
- [Transport trait](/quest/m2/cut-through/transport.md) - `web-transport-trait` carries unordered reads and offset writes, implemented for noq
- [Relay cut-through](/quest/m2/cut-through/relay.md) - lite and IETF group streams read out of order and write each range at its output offset

## Related

- [Unordered qmux](/quest/m2/p2p/unordered.md) - the same head-of-line problem on the data channel transport
- [Hierarchical stream scheduling](/quest/m1/quic/scheduler.md) - orders streams; offset writes order ranges within one
- [QoS](/quest/m1/qos/README.md) - the loss-delay counter follows its per-broadcast ingress row conventions
