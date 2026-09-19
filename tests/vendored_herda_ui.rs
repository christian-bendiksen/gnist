//! The vendored copy of `herda-ui` is Herda's, and stays Herda's.
//!
//! The crate is a byte-for-byte snapshot of `crates/herda-ui` in Herda, carried
//! here so that building Gnist needs no checkout of Herda, no credentials and no
//! live Git service — see `crates/herda-ui/UPSTREAM.md`. The cost of that is
//! that this repository cannot diff against upstream on its own.
//!
//! What it can do is notice that a file has been edited in place, which is the
//! failure that matters: a fix applied to this copy instead of upstream is
//! silently lost at the next refresh. `scripts/sync-herda-ui.sh` in Herda
//! writes the manifest below; this checks it.

use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/herda-ui")
}

/// The `| file | bytes | lines |` rows of UPSTREAM.md.
fn manifest(text: &str) -> Vec<(String, usize, usize)> {
    text.lines()
        .filter(|line| line.starts_with("| "))
        .filter_map(|line| {
            let mut cells = line.trim_matches('|').split('|').map(str::trim);
            let file = cells.next()?.to_owned();
            let bytes = cells.next()?.parse().ok()?;
            let lines = cells.next()?.parse().ok()?;
            Some((file, bytes, lines))
        })
        .collect()
}

#[test]
fn the_vendored_crate_is_the_one_that_was_synced() {
    let root = crate_root();
    let upstream = std::fs::read_to_string(root.join("UPSTREAM.md")).unwrap();

    let revision = upstream
        .lines()
        .find_map(|line| line.strip_prefix('`')?.strip_suffix("`."))
        .expect("UPSTREAM.md records the Herda commit this came from");
    assert_eq!(revision.len(), 40, "not a commit: {revision}");
    assert!(
        revision.chars().all(|c| c.is_ascii_hexdigit()),
        "not a commit: {revision}"
    );

    let recorded = manifest(&upstream);
    assert!(
        recorded.len() > 1,
        "UPSTREAM.md has no contents manifest; refresh it with \
         scripts/sync-herda-ui.sh from Herda"
    );

    for (file, bytes, lines) in &recorded {
        let path = root.join(file);
        let content =
            std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(
            content.len(),
            *bytes,
            "{file} has been edited in place; make the change in Herda and \
             refresh this copy with scripts/sync-herda-ui.sh"
        );
        assert_eq!(
            content.iter().filter(|byte| **byte == b'\n').count(),
            *lines,
            "{file} has been edited in place; make the change in Herda and \
             refresh this copy with scripts/sync-herda-ui.sh"
        );
    }

    // A file that is here but not in the manifest is drift the other way: a
    // module added only in this copy, or one the refresh did not remove.
    let mut present: Vec<String> = vec!["Cargo.toml".to_owned()];
    for entry in std::fs::read_dir(root.join("src")).unwrap() {
        let name = entry.unwrap().file_name();
        present.push(format!("src/{}", name.to_string_lossy()));
    }
    present.sort();
    let mut listed: Vec<String> = recorded.into_iter().map(|(file, ..)| file).collect();
    listed.sort();
    assert_eq!(
        present, listed,
        "the vendored tree and UPSTREAM.md disagree about which files are here"
    );
}
