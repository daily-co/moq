# [S] Fixed HLS target duration

## Goal

A `moq-hls` media playlist advertises one `EXT-X-TARGETDURATION` for the
whole run, as RFC 8216 requires, and never lists a segment whose `EXTINF`,
rounded to the nearest integer, exceeds it (RFC 8216 4.3.3.1).

## Plan

[#4280](https://github.com/moq-dev/moq/pull/4280) derives the target from the
current window's longest segment (`snapshot_as` in
`rs/moq-hls/src/export/rendition.rs`), so it rises when a long record arrives
and falls when that record is evicted. Codex flagged it
([r4113921580](https://github.com/moq-dev/moq/pull/4280#discussion_r4113921580)),
and the PR kept the observed maximum as recorded decision 4. The maintainer
has reversed that decision in the 09-28 merged-PR audit:

- Set the target once, from what the publisher declares: the catalog, or the
  GOP (keyframe interval) configuration where one is known. Not the catalog's
  `durationMax` blindly: it is a 10s split ceiling, and advertising it would
  push every player's live edge back by that much.
- A segment whose rounded `EXTINF` exceeds the target is refused or split,
  never listed as is. A 2.4s segment under a 2s target is valid and listed.
  Which one fits depends on where the long segment comes from (a reference
  record, or a non-reference rendition's snapped keyframe), so decide per
  case and say why.
- If the catalog declares nothing usable, decide whether to refuse the
  rendition or fix a target from the first segments and hold it; ask the
  maintainer if neither is clearly right.

Replace `target_duration_follows_the_observed_segments` with tests that pin a
constant target across a window whose segment durations vary, and that cover
the over-long segment path at the rounding boundary.

## Related

- [Per-track timelines](/quest/m1/archive/track-timeline/README.md) - the line this blocks
