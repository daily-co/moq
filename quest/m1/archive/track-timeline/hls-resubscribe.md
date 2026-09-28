# [S] HLS timeline resubscribe

## Goal

A transient error on a rendition's timeline subscription does not turn that
rendition's remaining segments into `EXT-X-GAP`. The watcher subscribes again
and keeps resolving segments; only a clean end of the timeline ends its spans.

## Plan

In [#4280](https://github.com/moq-dev/moq/pull/4280), `watch_spans` in
`rs/moq-hls/src/export/rendition.rs` logs any error and then calls
`spans.end()`, so rows past the last record resolve as gaps for good. Codex
flagged it as a P1
([r4112645622](https://github.com/moq-dev/moq/pull/4280#discussion_r4112645622)).
The agent declined it because the watcher never retries, and named
re-subscribing as the real fix. The maintainer's decision in the 09-28
merged-PR audit is to do that re-subscribe.

- Re-subscribe on an error while the broadcast is still live, resuming from
  the records the spans already hold rather than rebuilding them.
- Mark the spans ended only on a clean end of the timeline or when the
  broadcast itself goes away. The concern the decline raised still holds:
  nothing may park forever (`poll_resolved`, a recording
  `segments::Consumer`), so a rendition whose timeline never returns must
  still resolve.
- Re-subscribe on the event that makes it possible (the broadcast or track
  being available again), not on a timer.

Add a regression test where a non-reference rendition's timeline errors and
comes back, and its later segments resolve to media, not gaps.

## Related

- [Per-track timelines](/quest/m1/archive/track-timeline/README.md) - the line this blocks
