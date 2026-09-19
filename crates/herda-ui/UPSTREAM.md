# Herda terminal components

This is a source snapshot of `crates/herda-ui` from Herda commit
`da5581a4c4d5fa9d2d9cf8dafcf15dc77b9fb677`.

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
| Cargo.toml | 334 | 15 |
| src/browser.rs | 18222 | 510 |
| src/document.rs | 20779 | 593 |
| src/ink.rs | 11088 | 295 |
| src/lib.rs | 1881 | 37 |
| src/presentation.rs | 8993 | 256 |
| src/progress.rs | 22544 | 646 |
| src/terminal.rs | 21760 | 612 |
| src/width.rs | 9029 | 260 |
