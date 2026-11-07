// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A document is its content (`doc/plan.md`, "Identity"): followed across a move, told apart
//! from a copy, and a changed file is a new document at the same path.

use std::fs;
use std::path::Path;

use clerk_core::{EventKind, Index, hash_file};

fn setup() -> (tempfile::TempDir, tempfile::TempDir, Index) {
    let docs = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let index = Index::open(&elsewhere.path().join("index.sqlite")).unwrap();
    (docs, elsewhere, index)
}

fn write(root: &Path, name: &str, bytes: &[u8]) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn kinds(index: &Index, file: &Path) -> Vec<EventKind> {
    let hash = hash_file(file).unwrap();
    index
        .history(&hash)
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect()
}

#[test]
fn a_moved_document_keeps_its_history() {
    let (docs, _keep, mut index) = setup();
    let root = docs.path().canonicalize().unwrap();
    write(&root, "Downloads/statement.pdf", b"statement 1");
    write(&root, "notes.txt", b"notes");
    let first = index.scan(&root, |_| {}).unwrap();
    assert_eq!((first.files, first.hashed, first.added.len()), (2, 2, 2));

    let again = index.scan(&root, |_| {}).unwrap();
    assert_eq!(again.hashed, 0, "nothing changed, nothing read");
    assert!(again.added.is_empty() && again.removed.is_empty());

    fs::create_dir_all(root.join("Finanzen")).unwrap();
    fs::rename(
        root.join("Downloads/statement.pdf"),
        root.join("Finanzen/2026-10 statement.pdf"),
    )
    .unwrap();
    let moved = index.scan(&root, |_| {}).unwrap();
    assert_eq!(
        moved.moved,
        [(
            root.join("Downloads/statement.pdf"),
            root.join("Finanzen/2026-10 statement.pdf")
        )]
    );
    assert!(
        moved.added.is_empty() && moved.removed.is_empty(),
        "{moved:?}"
    );
    let now = root.join("Finanzen/2026-10 statement.pdf");
    assert_eq!(kinds(&index, &now), [EventKind::Added, EventKind::Moved]);
    assert_eq!(index.paths_of(&hash_file(&now).unwrap()).unwrap(), [now]);
}

#[test]
fn a_copy_a_change_and_a_removal_are_told_apart() {
    let (docs, _keep, mut index) = setup();
    let root = docs.path().canonicalize().unwrap();
    write(&root, "a.txt", b"one");
    write(&root, "b.txt", b"two");
    index.scan(&root, |_| {}).unwrap();

    fs::copy(root.join("a.txt"), root.join("a copy.txt")).unwrap();
    write(&root, "b.txt", b"two, edited");
    let report = index.scan(&root, |_| {}).unwrap();
    assert_eq!(
        report.copied,
        [(root.join("a.txt"), root.join("a copy.txt"))]
    );
    assert_eq!(report.changed, [root.join("b.txt")]);
    assert!(
        report.added.is_empty() && report.moved.is_empty(),
        "{report:?}"
    );
    assert_eq!(
        index
            .paths_of(&hash_file(&root.join("a.txt")).unwrap())
            .unwrap()
            .len(),
        2
    );

    fs::remove_file(root.join("a copy.txt")).unwrap();
    let report = index.scan(&root, |_| {}).unwrap();
    assert_eq!(report.removed, [root.join("a copy.txt")]);
    assert_eq!(
        kinds(&index, &root.join("a.txt")),
        [EventKind::Added, EventKind::Copied, EventKind::Removed]
    );
}

#[test]
fn find_matches_every_word_below_the_folder_ignoring_case() {
    let (docs, _keep, mut index) = setup();
    let root = docs.path().canonicalize().unwrap();
    write(&root, "Finanzen/Bank A/Kontoauszug_2026-10.pdf", b"x");
    write(&root, "Finanzen/Bank B/Kontoauszug_2026-10.pdf", b"y");
    write(&root, "Briefe/Kündigung.odt", b"z");
    index.scan(&root, |_| {}).unwrap();

    let found: Vec<_> = index
        .find(&["bank a", "KONTOAUSZUG"])
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(
        found,
        [root.join("Finanzen/Bank A/Kontoauszug_2026-10.pdf")]
    );
    assert_eq!(index.find(&["kündigung"]).unwrap().len(), 1);
    // The folder's own path is not searched: every file would match a word in it.
    let name = root.file_name().unwrap().to_string_lossy().to_string();
    assert!(index.find(&[&name]).unwrap().is_empty());
}

#[test]
fn a_folder_inside_one_already_scanned_is_covered_by_it() {
    let (docs, _keep, mut index) = setup();
    let root = docs.path().canonicalize().unwrap();
    write(&root, "Finanzen/a.txt", b"a");
    index.scan(&root.join("Finanzen"), |_| {}).unwrap();
    assert_eq!(index.roots().unwrap(), [root.join("Finanzen")]);
    index.scan(&root, |_| {}).unwrap();
    assert_eq!(
        index.roots().unwrap(),
        std::slice::from_ref(&root),
        "the outer one replaces it"
    );
    index.scan(&root.join("Finanzen"), |_| {}).unwrap();
    assert_eq!(index.roots().unwrap(), [root]);
}
