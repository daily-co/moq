# [M] Raw QUIC stream codes stay raw

## Goal

On a raw QUIC session (`moqt://`, `moql://`, raw iroh), RESET_STREAM and
STOP_SENDING carry the application's code as-is, not mapped through the
HTTP/3 WebTransport code space, so moq agrees with other raw QUIC MoQ stacks
on stream errors. WebTransport sessions keep the mapping they need.

## Plan

- The adapter fixes are up as moq-dev/noq#27 (`web-transport-moq`) and
  moq-dev/web-transport#408 (`web-transport-iroh`, `web-transport-quinn`).
  Raw sessions send codes as-is, read a plain code as-is, and still unmap a
  code in the HTTP/3 WebTransport range, which only an older raw peer sends.
  `moq-uring` already sent raw codes; it now unmaps that legacy range too.
- Remaining: release both adapters, bump the lock here (the `2.0` and `0.8`
  requirements already admit the patch releases), and the moq-tokio
  `stream_code.rs` test, which asserts a refused stream's code arrives
  verbatim over raw QUIC, goes green.
- Mixed versions: this is a wire break, so it stays on `dev`. An older peer
  reads a fixed peer's plain codes as invalid, which moq-net reports as
  `Error::Transport`. On moq-lite that costs only the stream: a cancel reads
  as a failure, a relay forwards INTERNAL_ERROR instead of the peer's code,
  and a relay HTTP fetch answers 500 instead of 404. On moq-transport, a
  release without #4602 ends the whole session when a stream is reset before
  its header, which a fixed peer does routinely. Release #4602 on `main`
  before any adapter release that sends raw codes.
- A `main`-compatible subset (read raw and legacy codes, keep sending mapped
  ones) could ship on `web-transport-moq` 1.3.x and `web-transport-iroh`
  0.7.x without breaking anyone, if interop needs it before `dev` releases.

Public API: none. Wire: raw QUIC stream error codes become the application's
own values.
