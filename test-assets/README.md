# test-assets

Shared fixtures, read by both the TypeScript and the Rust tests so the two
implementations cannot drift apart.

- `protocol/messages.jsonl`: one valid protocol message per line, covering every
  message type in `docs/PROTOCOL.md`.
- `protocol/waveform_chunk.hex`: one binary `waveform_chunk` frame as hex
  (id `wf1`, kind detail, offset 6, total 12, payload `01..06`).
- `sessions/demo.session`: the scripted mock session (timed mock commands).

Audio test files (click tracks, sine sweeps, multi-format copies) arrive with
Milestone 2 and will be generated, not checked in.
