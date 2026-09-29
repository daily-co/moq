# [S] Demo under a strict CSP

## Goal

Every production build of `demo/web` runs under a strict CSP and loads the
worklets and capture worker from its own origin. Watch, publish, and meet play
and publish audio in Chromium and Firefox with no CSP violations. `vite dev`
keeps the blob: defaults.

## Plan

Decided in planning (the paper trail is on
[#4518](https://github.com/moq-dev/moq/pull/4518)):

- Strict CSP is the default for `vite build` and `vite preview`, so every
  preview or deploy dogfoods hosted mode. Dev stays on blob, because HMR and
  the dev worklets need it. A `?csp` opt-in was rejected, since it goes
  untested unless someone remembers to use it.
- Inject the policy as a `<meta http-equiv="Content-Security-Policy">` from
  a build-only `transformIndexHtml` plugin, so it travels with the HTML on any
  host. Directives: `script-src 'self' 'wasm-unsafe-eval'` and
  `worker-src 'self'`. The wasm allowance is for the libav Opus polyfill
  (`js/hang/src/util/libav.ts`), which loads only when `AudioEncoder` is
  missing. Check whether the polyfill also spawns blob: workers; if so, name
  that gap in the PR rather than loosening the CSP.
- Pages call `Watch.assets("/assets/")` and `Publish.assets("/assets/")` when
  `import.meta.env.PROD`. The demo's own build already emits the files there
  through `vite-plugin-worklet`, so nothing needs copying.
- Verify manually in Chromium and Firefox: watch playing audio, publish
  capturing mic and camera (Firefox takes the capture worker), and meet. There
  should be no `securitypolicyviolation` events. Report the results in the PR.

## Related

- [Plan: watch worker](/quest/m1/plan-watch-worker.md) - a new watch worker must load under this CSP too
