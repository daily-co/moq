# [XS] Tracing captures survive the call-site cache

## Goal

No test in the workspace counts or asserts tracing events through a
thread-scoped subscriber (`with_default`, `set_default`) that a parallel
test can silence under a shared-process `cargo test`.

## Plan

#4690 found that tracing caches, per call site and for the whole process,
whether any subscriber wants its events. With only scoped subscribers, the
first thread to hit a call site decides for everyone, so a parallel test on a
thread with no subscriber turns the event off and the capture misses it.
Nextest runs each test in its own process and can't hit it, but `cargo test`
does.

Grep the Rust crates for tests that capture tracing output with scoped
subscribers. Move each one to the helper #4690 added in
`rs/moq-net/src/model/test_tracing.rs` (one process-wide subscriber, events
routed to a per-thread counter), or a shared copy of it if other crates need
it. If none turn up, delete this quest.

Public API: none. Wire: none.
