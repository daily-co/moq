# [L] Offset writes in the noq fork

## Goal

A send stream in moq-dev/noq accepts a write at an offset past unwritten
bytes, sends it right away, and fills the gap when those bytes are written
later. A released `moq-noq-proto` and `moq-noq` carry it.

## Plan

Decided 2026-09-30: no write-ahead limit beyond QUIC's own flow control.

- An offset write reserves the gap and queues the range. Stream and connection
  credit is charged up to the highest written offset, as it already is on the
  receiver's side.
- Within a stream, lower offsets are sent first once they are written, and
  retransmits keep the existing stream priority path.
- `finish` waits until every range below the final size is written; a write
  that overlaps an existing range is an error.
- The append-only `write` and `write_chunks` stay as they are; the offset write
  is a new method, since a writer that never skips pays nothing.
- Tests in noq-proto's simulated pair: an out-of-order writer delivers the same
  bytes as an ordered one, flow control blocks at the highest offset, FIN waits
  for the gap, and a reset with a gap outstanding is clean.

Release the fork crates with the change and record the parent noq commit, per
the [QUIC line's rules](/quest/m1/quic/README.md). Offer it upstream once it has
shipped and settled, like the fork's other m2 features.

## Required

- [Bench](/quest/m2/cut-through/bench.md) - a go verdict
