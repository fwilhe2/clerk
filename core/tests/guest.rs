// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! C1's guard (`doc/plan.md`): everything clerk does unasked leaves the folder exactly as it
//! was — every entry, hidden ones included, with its size, modification time and permissions.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use clerk_core::{Error, Index, hash_file};

mod common;

#[derive(Debug, PartialEq)]
struct Entry {
    path: PathBuf,
    dir: bool,
    len: u64,
    modified: SystemTime,
    readonly: bool,
}

fn snapshot(root: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let meta = fs::symlink_metadata(entry.path()).unwrap();
            if meta.is_dir() {
                stack.push(entry.path());
            }
            out.push(Entry {
                path: entry.path().strip_prefix(root).unwrap().to_owned(),
                dir: meta.is_dir(),
                len: meta.len(),
                modified: meta.modified().unwrap(),
                readonly: meta.permissions().readonly(),
            });
        }
    }
    let meta = fs::metadata(root).unwrap();
    out.push(Entry {
        path: PathBuf::new(),
        dir: true,
        len: 0,
        modified: meta.modified().unwrap(),
        readonly: meta.permissions().readonly(),
    });
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn documents() -> tempfile::TempDir {
    let docs = tempfile::tempdir().unwrap();
    let root = docs.path();
    fs::create_dir_all(root.join("Finanzen/Bank A")).unwrap();
    fs::create_dir_all(root.join(".hidden")).unwrap();
    fs::write(
        root.join("Finanzen/Bank A/Kontoauszug_1_2026-10.pdf"),
        b"%PDF statement",
    )
    .unwrap();
    fs::write(root.join("letter.txt"), b"Dear reader").unwrap();
    fs::write(root.join(".hidden/secret"), b"left alone").unwrap();
    let locked = root.join("locked.txt");
    fs::write(&locked, b"read-only").unwrap();
    let mut perms = fs::metadata(&locked).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&locked, perms).unwrap();
    docs
}

#[test]
fn scanning_finding_and_asking_leave_the_folder_as_it_was() {
    let docs = documents();
    // A scan and a picture, so OCR runs too, where it is installed.
    let mut extra = 0;
    if let (Some(scan), Some(picture)) = (
        common::scanned_pdf(&["Bescheid ueber Grundsteuer"]),
        common::picture(&["Garantieschein"], "-png"),
    ) {
        fs::write(docs.path().join("scan.pdf"), scan).unwrap();
        fs::write(docs.path().join("receipt.png"), picture).unwrap();
        extra = 2;
    }
    let elsewhere = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    fs::write(
        downloads.path().join("Kontoauszug_2_2026-11.pdf"),
        b"%PDF new",
    )
    .unwrap();
    fs::write(downloads.path().join("letter copy.txt"), b"Dear reader").unwrap();
    let before = snapshot(docs.path());
    let inbox_before = snapshot(downloads.path());

    let mut index = Index::open(&elsewhere.path().join("index.sqlite")).unwrap();
    let first = index.scan(docs.path(), |_| {}).unwrap();
    assert_eq!(
        first.files,
        3 + extra,
        "the hidden file is left out: {first:?}"
    );
    index.scan(docs.path(), |_| {}).unwrap();
    index.find(&["kontoauszug"]).unwrap();
    index.status().unwrap();
    let hash = hash_file(&docs.path().join("letter.txt")).unwrap();
    index.paths_of(&hash).unwrap();
    index.history(&hash).unwrap();
    let advice = index.inbox(downloads.path()).unwrap();
    assert_eq!(advice.len(), 2);
    drop(index);

    assert_eq!(snapshot(docs.path()), before);
    assert_eq!(snapshot(downloads.path()), inbox_before);
}

#[test]
fn an_index_inside_the_folder_is_refused_before_anything_is_read() {
    let docs = documents();
    let file = docs.path().join("index.sqlite");
    let mut index = Index::open(&file).unwrap();
    let refused = index.scan(docs.path(), |_| {});
    assert!(
        matches!(refused, Err(Error::IndexInside { .. })),
        "{refused:?}"
    );
    assert_eq!(index.status().unwrap().files, 0);
}
