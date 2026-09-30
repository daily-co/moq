# [L] Relay cut-through for group streams

## Goal

The lite and IETF sessions read incoming group streams out of order and write
each payload range to every subscriber's stream at its output offset, so bytes
behind an ingress hole reach the next hop before the hole is filled. On the
[bench](/quest/m2/cut-through/bench.md), frame completion moves toward the
direct-connection bound.

## Plan

Decided 2026-09-30: always on when the ingress and egress streams both support
it, no config knob; otherwise today's ordered path.

- Ingest: read the stream header in order, then switch to unordered reads.
  Frame headers are parsed from the contiguous prefix; once a header is known,
  its frame's payload ranges go into the frame with the
  [offset write](/quest/m2/cut-through/frames.md) wherever they land. A hole
  covering the next header blocks later frames, because that header's position
  depends on it. Lite: `FrameIngest` in `lite/subscriber.rs`; IETF:
  `GroupIngest` in `ietf/subscriber.rs`. `Reader::poll_read_frame` in
  `coding/reader.rs` is the ordered path they share today.
- Egress: each subscriber's output offset for a payload range is its stream
  prefix plus every earlier frame's encoded header and size, plus this frame's
  header, plus the range offset. That is known once every earlier input header
  has been parsed, including across a lite and IETF translation, since output
  header lengths depend on values (timestamp and object id deltas). Write the
  header, then the ranges with the
  [offset write](/quest/m2/cut-through/transport.md). Lite: `GroupServe` in
  `lite/publisher.rs`; IETF: `GroupServe` in `ietf/publisher.rs`.
- The fetch paths stay ordered.
- Tests over the mock transport with shuffled chunk delivery: every subscriber
  version receives byte-identical streams to the ordered path, and a
  header-covering hole holds later frames.

Re-run the bench and record the result in the line's README.

## Required

- [Frame ranges](/quest/m2/cut-through/frames.md) - the model primitive
- [Transport trait](/quest/m2/cut-through/transport.md) - the I/O primitive
