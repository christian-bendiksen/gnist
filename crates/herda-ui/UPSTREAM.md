# Herda terminal components

This is a source snapshot of `crates/herda-ui` from Herda commit
`46c55ab77998ad5500e1e78aa9afcd9eac5ba41d`.

The implementation belongs to Herda. Keep its Cargo.toml and src/ byte-for-byte
identical when updating this snapshot. Make renderer changes upstream, then
refresh this directory with `scripts/sync-herda-ui.sh` there.

Gnist includes this small crate so building Gnist does not need a separate Git
checkout, credentials for Herda, or a live Git service. There is no runtime
service, process or dependency on pkg or Malm. Theme resolution, rendering and
the transaction stay in Gnist; all three CLIs use the same terminal palette,
glyphs, width ladder and document grammar.

It costs about 38 KiB of the stripped release binary, measured against the same
build without it. Gnist is tuned for size — `opt-level = "z"`, LTO,
`panic = "abort"` — so that was checked before adopting the crate rather than
assumed.

## Contents

Written by `scripts/sync-herda-ui.sh` in Herda. A test in this
repository fails if a file below has been edited in place; fix it
upstream and refresh instead.

| file | bytes | lines |
| --- | --- | --- |
| Cargo.toml | 1523 | 36 |
| src/browser.rs | 24778 | 685 |
| src/cells.rs | 44854 | 1212 |
| src/document.rs | 21345 | 613 |
| src/frame_fixtures.rs | 22190 | 657 |
| src/frame.rs | 28615 | 815 |
| src/frame_tests.rs | 23352 | 620 |
| src/ink.rs | 16386 | 436 |
| src/lib.rs | 3569 | 76 |
| src/pieces.rs | 76519 | 2207 |
| src/presentation.rs | 9450 | 265 |
| src/progress.rs | 24338 | 694 |
| src/revision_line.rs | 46239 | 1204 |
| src/scale.rs | 14547 | 352 |
| src/screen_tests.rs | 5666 | 201 |
| src/session.rs | 29354 | 763 |
| src/session_tests.rs | 8594 | 291 |
| src/surface.rs | 5381 | 151 |
| src/surface_tests.rs | 1975 | 65 |
| src/terminal.rs | 25204 | 690 |
| src/width.rs | 9496 | 269 |
| src/work.rs | 23172 | 659 |
