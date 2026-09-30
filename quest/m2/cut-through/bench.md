# [M] Benchmark head-of-line blocking at a relay hop

## Goal

A nightly bench measures how long a frame takes to complete at the viewer when
the publisher's hop to the relay loses packets, and how much of that a
cut-through relay could win back. It ends with a written go or no-go for the
rest of the [cut-through line](/quest/m2/cut-through/README.md).

## Plan

Decided 2026-09-30: real stacks, not a mock. The loss recovery being measured
belongs to noq, so a mock transport with a made-up retransmit model would
partly measure its own assumptions.

- Publisher, then a seeded `moq-shaper` (loss and delay), then `moq-relay` over
  noq, then a clean or rate-limited hop to the viewer. Reuse the shaper wiring
  from `rs/moq-relay/tests/drills.rs`.
- Sweep loss rate, frame size (audio-sized through a large I-frame), and egress
  headroom (egress rate relative to the media rate).
- Report per-frame completion at the viewer, from publish to last byte, p50 and
  p99. The bound is the same path with the relay replaced by a direct
  publisher-to-viewer connection over the same shaper: roughly what perfect
  cut-through approaches. The gap between relay and direct is the opportunity.
- Record the loss-delay counter from the same runs, if
  [Loss delay](/quest/m2/cut-through/loss-delay.md) has landed, so production
  numbers can be read against the lab.
- It runs on wall-clock time, so it is a nightly bench, not a unit test. Wire it
  into the nightly workflow.

Write the numbers and the verdict into this line's README. The maintainer makes
the go or no-go call from them; no threshold is set in advance. On a no-go,
delete the build quests and keep the bench.
