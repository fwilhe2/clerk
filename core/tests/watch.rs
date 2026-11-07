// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! M6 (`doc/plan.md`): while clerk watches, a document added, moved or hidden is in the index
//! without a scan being asked for.

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use clerk_core::{Index, Report};

#[test]
fn a_change_while_watching_is_scanned_on_its_own() {
    let docs = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let root = docs.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("Finanzen")).unwrap();
    let mut index = Index::open(&elsewhere.path().join("index.sqlite")).unwrap();
    index.scan(&root, |_| {}).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<Report>();
    let watching = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            index
                .watch(
                    || stop.load(Ordering::Relaxed),
                    |report| tx.send(report.unwrap()).unwrap(),
                )
                .unwrap();
            index
        })
    };
    let next = || rx.recv_timeout(Duration::from_secs(20)).expect("a scan");
    let first = next();
    assert!(
        first.added.is_empty(),
        "nothing new at the start: {first:?}"
    );

    fs::write(root.join("Kontoauszug_1.pdf"), b"statement").unwrap();
    let added = next();
    assert_eq!(added.added, [root.join("Kontoauszug_1.pdf")], "{added:?}");

    fs::rename(
        root.join("Kontoauszug_1.pdf"),
        root.join("Finanzen/Kontoauszug_1.pdf"),
    )
    .unwrap();
    let moved = next();
    assert_eq!(moved.moved.len(), 1, "{moved:?}");

    fs::create_dir_all(root.join(".cache")).unwrap();
    fs::write(root.join(".cache/lock"), b"x").unwrap();
    assert!(
        rx.recv_timeout(Duration::from_secs(3)).is_err(),
        "a hidden change is no reason to scan"
    );

    stop.store(true, Ordering::Relaxed);
    let index = watching.join().unwrap();
    assert_eq!(index.find(&["kontoauszug"]).unwrap().len(), 1);
}
