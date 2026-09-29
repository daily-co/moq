# [M] Timelines declare their segment duration

## Goal

Each timeline in the catalog `archive` map declares its own maximum segment
duration, reported by the application like bitrate or framerate, or estimated
by the publisher when the application doesn't know it. The broadcast-wide
`durationMax`, filled from moq-mux's 10s default, is deleted. Every edge reads
the same declared value, so an HLS edge can fix its target duration from it.

## Plan

Decided in planning (09-29), after the hls-target agent found that nothing in
the catalog declares a usable target:

- **Shape.** Each `timelines` entry becomes an object:
  `timelines: { video: { track: "video.timeline.z", duration: 2000 } }`, in
  the archive `timescale`. The root `durationMax` goes away. This is a break in
  place on this line, with no compatibility path. Update `rs/hang`,
  `drafts/draft-lcurley-moq-hang.md`, and every `durationMax` reference
  (grep it, including `doc/`). The JS port happens in
  [JS per-track timelines](/quest/m1/archive/track-timeline/js.md).
- **Meaning: a nominal target, not a hard max.** The segmenter cuts at the
  group boundary nearest the declared duration, so records start on keyframes.
  A GOP longer than the target overruns, and moq-hls lists the overrun with a
  warning. A mid-group split remains only as a safety ceiling at a multiple of
  the target (pick one, for example 3x), which is internal and not declared.
  A hard max that splits at the target was rejected: jitter makes slivers, and
  segments would start without a keyframe.
- **Reported by the application.** moq-mux takes an optional per-track
  duration hint. moq-video's encoder sets it from its `encode::Gop`. Importers
  pass nothing and estimate.
- **Estimated by the publisher when not reported.** moq-mux holds that
  timeline's catalog entry until the first complete group, then declares its
  duration rounded up to the timescale. If no second keyframe arrives within
  the safety ceiling, it declares the ceiling. Estimating at the edge was
  rejected because edges and restarts would disagree. A rolling estimate was
  rejected because it conflicts with a fixed target. The user chose the first
  group over the longer of the first two groups, accepting that an irregular
  first GOP sets the value.
- **Fixed per timeline.** The declared value never changes for a timeline
  track's life. A reconfigure that makes a new timeline track declares its own.
- Sparse timelines (the catalog's own, zero minimum) keep their current
  cutting. Decide what, if anything, they declare, and keep it out of HLS.

Tests: a hinted track declares its hint. An unhinted track holds its entry
until the first group completes, then declares that group's duration. A first
group that never closes declares the ceiling. The cut lands on the boundary
nearest the target. The declared value survives later, longer groups.

## Related

- [Fixed HLS target duration](/quest/m1/archive/track-timeline/hls-target.md) - consumes the declared duration
