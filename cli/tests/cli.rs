// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `clerk` binary, end to end: scan, find, where and status.

use std::fs;
use std::process::{Command, Output};

fn clerk(index: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_clerk"))
        .arg("--index")
        .arg(index)
        .args(args)
        .output()
        .expect("the binary runs")
}

fn ok(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn a_document_is_found_and_followed_after_a_move() {
    let docs = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let index = elsewhere.path().join("index.sqlite");
    let root = docs.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("Finanzen")).unwrap();
    fs::write(root.join("Kontoauszug_2026-10.pdf"), b"%PDF").unwrap();
    let root_text = root.to_str().unwrap();

    let first = ok(clerk(&index, &["scan", root_text]));
    assert!(first.contains("1 files, 1 read; 1 added"), "{first}");

    fs::rename(
        root.join("Kontoauszug_2026-10.pdf"),
        root.join("Finanzen/Kontoauszug_2026-10.pdf"),
    )
    .unwrap();
    let rescan = ok(clerk(&index, &["scan", "--verbose"]));
    assert!(
        rescan.contains("1 moved"),
        "the folder is remembered: {rescan}"
    );

    let found = ok(clerk(&index, &["find", "kontoauszug", "2026"]));
    assert_eq!(
        found.trim(),
        root.join("Finanzen/Kontoauszug_2026-10.pdf")
            .to_str()
            .unwrap()
    );

    // A copy outside the folder is recognised by its content.
    let copy = elsewhere.path().join("copy.pdf");
    fs::write(&copy, b"%PDF").unwrap();
    let place = ok(clerk(&index, &["where", copy.to_str().unwrap()]));
    assert!(
        place.contains("\tadded\t") && place.contains("\tmoved\t"),
        "{place}"
    );

    let status = ok(clerk(&index, &["status"]));
    assert!(
        status.contains("files\t1") && status.contains(root_text),
        "{status}"
    );

    let unknown = elsewhere.path().join("unknown.pdf");
    fs::write(&unknown, b"never seen").unwrap();
    assert!(
        !clerk(&index, &["where", unknown.to_str().unwrap()])
            .status
            .success()
    );
}
