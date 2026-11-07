// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! C5's guard (`doc/plan.md`): a move is the person's, never replaces a file, crosses to another
//! file system only when told to, and can be undone.

use std::fs;
use std::path::{Path, PathBuf};

use clerk_core::{Error, EventKind, Index, hash_file};

struct Setup {
    _dir: tempfile::TempDir,
    docs: PathBuf,
    downloads: PathBuf,
    index: Index,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let docs = root.join("Documents");
    let downloads = root.join("Downloads");
    fs::create_dir_all(docs.join("Finanzen")).unwrap();
    fs::create_dir_all(&downloads).unwrap();
    fs::write(docs.join("Finanzen/Kontoauszug_1.pdf"), b"statement 1").unwrap();
    let mut index = Index::open(&root.join("index.sqlite")).unwrap();
    index.scan(&docs, |_| {}).unwrap();
    Setup {
        _dir: dir,
        docs,
        downloads,
        index,
    }
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

#[test]
fn a_move_is_journalled_indexed_and_undone() {
    let Setup {
        docs,
        downloads,
        mut index,
        _dir: _keep,
    } = setup();
    let new = downloads.join("Kontoauszug_2.pdf");
    fs::write(&new, b"statement 2").unwrap();

    let done = index
        .move_file(&new, &docs.join("Finanzen"), false)
        .unwrap();
    let there = docs.join("Finanzen/Kontoauszug_2.pdf");
    assert_eq!(
        (done.from.as_path(), done.to.as_path()),
        (new.as_path(), there.as_path())
    );
    assert!(!new.exists());
    assert_eq!(read(&there), b"statement 2");
    assert!(!done.copied);
    assert_eq!(index.journal().unwrap(), std::slice::from_ref(&done));

    let found: Vec<PathBuf> = index
        .find(&["kontoauszug_2"])
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(
        found,
        std::slice::from_ref(&there),
        "indexed at once, before any scan"
    );
    let hash = hash_file(&there).unwrap();
    let history = index.history(&hash).unwrap();
    assert_eq!(history.last().unwrap().kind, EventKind::Moved);
    assert_eq!(history.last().unwrap().from.as_deref(), Some(new.as_path()));
    let rescan = index.scan(&docs, |_| {}).unwrap();
    assert!(
        rescan.added.is_empty() && rescan.moved.is_empty(),
        "{rescan:?}"
    );

    let undone = index.undo().unwrap();
    assert_eq!(undone.id, done.id);
    assert!(undone.undone.is_some());
    assert_eq!(read(&new), b"statement 2");
    assert!(!there.exists());
    assert!(index.find(&["kontoauszug_2"]).unwrap().is_empty());
    assert!(matches!(index.undo(), Err(Error::NothingToUndo)));
    assert!(index.journal().unwrap()[0].undone.is_some());
}

#[test]
fn a_move_never_replaces_a_file() {
    let Setup {
        docs,
        downloads,
        mut index,
        _dir: _keep,
    } = setup();
    let new = downloads.join("Kontoauszug_1.pdf");
    fs::write(&new, b"another statement 1").unwrap();

    let refused = index.move_file(&new, &docs.join("Finanzen"), true);
    assert!(matches!(refused, Err(Error::Exists(_))), "{refused:?}");
    assert_eq!(read(&new), b"another statement 1");
    assert_eq!(
        read(&docs.join("Finanzen/Kontoauszug_1.pdf")),
        b"statement 1"
    );
    assert!(index.journal().unwrap().is_empty());
}

#[test]
fn undo_leaves_a_changed_file_and_a_taken_place_alone() {
    let Setup {
        docs,
        downloads,
        mut index,
        _dir: _keep,
    } = setup();
    let new = downloads.join("letter.txt");
    fs::write(&new, b"letter").unwrap();
    index.move_file(&new, &docs, false).unwrap();
    let there = docs.join("letter.txt");

    fs::write(&there, b"letter, edited").unwrap();
    assert!(matches!(index.undo(), Err(Error::Changed(_))));
    assert_eq!(read(&there), b"letter, edited");

    fs::write(&there, b"letter").unwrap();
    fs::write(&new, b"a new letter by the same name").unwrap();
    assert!(matches!(index.undo(), Err(Error::Exists(_))));
    assert_eq!(read(&there), b"letter");
    assert_eq!(read(&new), b"a new letter by the same name");
}

#[cfg(unix)]
#[test]
fn a_move_to_another_file_system_waits_to_be_told() {
    use std::os::unix::fs::MetadataExt;
    let Setup {
        downloads,
        mut index,
        _dir: _keep,
        ..
    } = setup();
    let Ok(other) = tempfile::tempdir_in("/dev/shm") else {
        eprintln!("skipped: no /dev/shm");
        return;
    };
    if fs::metadata(other.path()).unwrap().dev() == fs::metadata(&downloads).unwrap().dev() {
        eprintln!("skipped: /dev/shm is the same file system as the temporary directory");
        return;
    }
    let new = downloads.join("scan.pdf");
    fs::write(&new, b"scan").unwrap();
    let modified = fs::metadata(&new).unwrap().modified().unwrap();

    let refused = index.move_file(&new, other.path(), false);
    assert!(
        matches!(refused, Err(Error::AcrossFileSystems { .. })),
        "{refused:?}"
    );
    assert_eq!(read(&new), b"scan");

    let done = index.move_file(&new, other.path(), true).unwrap();
    assert!(done.copied);
    assert!(!new.exists());
    assert_eq!(read(&done.to), b"scan");
    assert_eq!(
        fs::metadata(&done.to).unwrap().modified().unwrap(),
        modified
    );

    index.undo().unwrap();
    assert_eq!(read(&new), b"scan");
    assert!(!done.to.exists());
}
