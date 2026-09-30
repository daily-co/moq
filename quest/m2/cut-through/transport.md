# [M] Unordered reads and offset writes in web-transport-trait

## Goal

A released `web-transport-trait` lets a receive stream yield chunks with their
offsets in arrival order and lets a send stream write at an offset. The noq
backends implement both; every other backend reports that it cannot.

## Plan

Decided 2026-09-30: additive methods with defaults, not a moq-net-private
extension trait, which would need per-session detection behind the generic
`Session`.

- `RecvStream` gains an unordered chunk read returning `(offset, Bytes)`, and
  `SendStream` gains an offset write, both in the poll style. Their defaults
  report unsupported in the return type (as
  [poll_acked](/quest/m2/quic-ack-hook.md) does, since a default cannot build
  `Self::Error`), and the caller falls back to ordered I/O.
- noq allows no ordered read after an unordered one
  (`moq-noq-proto` `assembler.rs`), so the unordered mode is entered once per
  stream and never left.
- Implement both in `web-transport-moq` (released from the fork) over
  `RecvStream::into_unordered` and the [offset write](/quest/m2/cut-through/noq.md),
  and natively in `rs/moq-uring/src/quic/noq/stream.rs`, which today reads with
  `read(true)` and drops the chunk offset. moq-tokio's poll adapter forwards
  them. `web-transport-wasm`, qmux, and iroh keep the defaults.
- Bump the pins here.

## Required

- [Offset writes in noq](/quest/m2/cut-through/noq.md) - the send-side primitive this exposes
