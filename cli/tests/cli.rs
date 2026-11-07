// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `clerk` binary, end to end.

use std::fs;
use std::process::{Command, Output};

#[path = "../../core/tests/common/mod.rs"]
mod common;

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

    fs::write(
        root.join("Finanzen/Nebenkosten.txt"),
        "Abrechnung der Nebenkosten 2025\nNachzahlung: 120 EUR\n",
    )
    .unwrap();
    let scanned = ok(clerk(&index, &["scan"]));
    assert!(scanned.contains("2 with text"), "{scanned}");
    let found = ok(clerk(&index, &["find", "--context", "nachzahlung"]));
    let mut lines = found.lines();
    assert_eq!(lines.next(), root.join("Finanzen/Nebenkosten.txt").to_str());
    assert!(lines.next().unwrap().contains("[Nachzahlung]"), "{found}");
    let text = ok(clerk(
        &index,
        &[
            "text",
            root.join("Finanzen/Nebenkosten.txt").to_str().unwrap(),
        ],
    ));
    assert_eq!(
        text,
        "Abrechnung der Nebenkosten 2025\nNachzahlung: 120 EUR\n"
    );
    assert!(ok(clerk(&index, &["status"])).contains("with text\t2"));

    let unknown = elsewhere.path().join("unknown.pdf");
    fs::write(&unknown, b"never seen").unwrap();
    assert!(
        !clerk(&index, &["where", unknown.to_str().unwrap()])
            .status
            .success()
    );
}

#[test]
fn a_scan_waits_for_ocr_and_is_read_once_it_is_installed() {
    if !common::ocr_installed() {
        return;
    }
    let docs = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let index = elsewhere.path().join("index.sqlite");
    let root = docs.path().canonicalize().unwrap();
    let scan = root.join("scan.pdf");
    fs::write(
        &scan,
        common::scanned_pdf(&["Kfz Versicherung Beitragsrechnung"]).unwrap(),
    )
    .unwrap();
    let scan = scan.to_str().unwrap();

    let without = Command::new(env!("CARGO_BIN_EXE_clerk"))
        .arg("--index")
        .arg(&index)
        .args(["scan", root.to_str().unwrap()])
        .env("PATH", elsewhere.path())
        .output()
        .unwrap();
    let report = ok(without);
    assert!(
        report.contains("1 scans or pictures to read once tesseract is installed"),
        "{report}"
    );
    assert!(ok(clerk(&index, &["status"])).contains("awaiting OCR\t1"));
    let waiting = clerk(&index, &["text", scan]);
    assert!(String::from_utf8_lossy(&waiting.stderr).contains("install tesseract"));

    let with = ok(clerk(&index, &["scan"]));
    assert!(with.contains("1 with text"), "{with}");
    assert!(ok(clerk(&index, &["text", scan])).contains("Beitragsrechnung"));
    assert!(ok(clerk(&index, &["find", "versicherung"])).contains("scan.pdf"));
}

#[test]
fn inbox_suggests_a_folder_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let index = root.join("index.sqlite");
    let bank = root.join("Documents/Finanzen/Bank A");
    fs::create_dir_all(&bank).unwrap();
    for n in 1..=3 {
        fs::write(
            bank.join(format!("Kontoauszug_{n}_2026-0{n}.txt")),
            format!("{n}"),
        )
        .unwrap();
    }
    let downloads = root.join("Downloads");
    fs::create_dir_all(&downloads).unwrap();
    fs::write(downloads.join("Kontoauszug_4_2026-04.txt"), "4").unwrap();
    fs::write(downloads.join("Kontoauszug_1 copy.txt"), "1").unwrap();
    ok(clerk(
        &index,
        &["scan", root.join("Documents").to_str().unwrap()],
    ));

    let said = ok(clerk(&index, &["inbox", downloads.to_str().unwrap()]));
    assert!(
        said.contains(&format!(
            "Kontoauszug_4_2026-04.txt\t→ {}\n",
            bank.display()
        )),
        "{said}"
    );
    assert!(
        said.contains("\tall 3 files named Kontoauszug_<n>_<date>.txt are there"),
        "{said}"
    );
    assert!(
        said.contains("Kontoauszug_1 copy.txt\tis already at"),
        "{said}"
    );

    let new = downloads.join("Kontoauszug_4_2026-04.txt");
    let copy = downloads.join("Kontoauszug_1 copy.txt");
    let filed = ok(clerk(
        &index,
        &["file", new.to_str().unwrap(), copy.to_str().unwrap()],
    ));
    assert!(filed.contains("moved\t"), "{filed}");
    assert!(filed.contains("left\t"), "the copy stays: {filed}");
    assert!(bank.join("Kontoauszug_4_2026-04.txt").exists() && !new.exists());
    assert!(copy.exists());
    let journal = ok(clerk(&index, &["journal"]));
    assert_eq!(journal.lines().count(), 1, "{journal}");

    let undone = ok(clerk(&index, &["undo"]));
    assert!(undone.starts_with("moved back\t"), "{undone}");
    assert!(new.exists() && !bank.join("Kontoauszug_4_2026-04.txt").exists());
    assert!(ok(clerk(&index, &["journal"])).contains("\tundone "));
    assert!(
        !clerk(&index, &["undo"]).status.success(),
        "nothing left to undo"
    );

    fs::write(bank.join("Kontoauszug_4_2026-04.txt"), "someone else's").unwrap();
    let refused = clerk(
        &index,
        &[
            "move",
            new.to_str().unwrap(),
            "--to",
            bank.to_str().unwrap(),
        ],
    );
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("does not replace"));
    assert_eq!(fs::read_to_string(&new).unwrap(), "4");
}
