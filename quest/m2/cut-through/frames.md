# [M] Frames fill out of order

## Goal

A `moq-net` frame accepts payload ranges at any offset, and a reader can
consume each range as soon as it lands. Existing in-order readers and writers
behave exactly as before.

## Plan

Decided 2026-09-30:

- `frame::Producer` gains an offset write and `frame::Consumer` gains a read
  that yields `(offset, Bytes)` ranges in arrival order. Both are
  `pub(crate)`: the only users are the lite and IETF sessions in the same crate.
- A frame written out of order keeps the received chunks by offset instead of
  copying into a pre-allocated buffer, so its memory tracks bytes received and
  [the allocation budget](/quest/m0/frame-alloc-budget.md) does not apply. The
  in-order readers (`poll_read_chunk`, `poll_read_all`) still see contiguous
  bytes up to the first hole, and `finish` still requires every byte.
- Overlapping or out-of-bounds ranges are errors.
- Today's single sequential writer (`model/frame.rs`, the one `written`
  watermark and its safety comments) is the invariant being changed; keep the
  ordered fast path, including the whole-frame zero-copy install, unchanged.

Extend `rs/moq-net/benches/group.rs` so the ordered path shows no regression,
and test that a frame filled in shuffled ranges reads back identical through
both readers.

## Required

- [Bench](/quest/m2/cut-through/bench.md) - a go verdict
