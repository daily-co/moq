# [S] moq-mux live import tests on the paused clock

## Goal

The moq-mux live import restart tests (`live_import_restarts_forward_after_idle`
for ts, fmp4, and flv) advance a paused clock across their idle gap and sleep
no real time.

## Plan

Facts (`origin/main`, 2026-10-01): the helpers (`live_import` in
`container/{ts,fmp4}/import_test.rs`, `import` in `flv/import_test.rs`) call
`std::thread::sleep(idle)` with a 300 ms idle. The gap is measured through
`clock::Anchor`, which reads `crate::Clock::now()`, and that is
`std::time::Instant::elapsed()`, so a paused tokio clock can't move it.

Decided (2026-10-01): `crate::Clock`'s monotonic epoch becomes a
`web_async::time::Instant`. That is tokio's clock on native, so the tests start
paused and call `tokio::time::advance`, and it takes `std::time::Instant` off the
wasm path, as #4687 did for the TS SI debounce. The wall mapping stays
`SystemTime`. Rejected: injecting a `now` into the importers through
`translate_at`, which is plumbing only tests want.

Update `test_util::late_clock` to match, and check other `crate::Clock` users
(including `Clock::at` callers) still build on wasm.

Public API: check whether `Clock::at` is exported; if its `Instant` type
changes, report it. Wire: none.
