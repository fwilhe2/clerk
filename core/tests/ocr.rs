// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! M3 (`doc/plan.md`): a scanned letter is found by a word in it, read by OCR into the index
//! and never into the file. Needs `tesseract` and `pdftoppm`, and passes with a notice without.

use std::fs;

use clerk_core::{DocumentText, Index, hash_file};

mod common;

#[test]
fn a_scanned_letter_is_found_by_a_word_in_it() {
    if !common::ocr_installed() {
        return;
    }
    let docs = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let root = docs.path().canonicalize().unwrap();
    let letter = [
        "Stadtwerke Musterstadt",
        "Jahresabrechnung Fernwaerme",
        "Guthaben 87 EUR",
    ];
    fs::write(
        root.join("scan_0001.pdf"),
        common::scanned_pdf(&letter).unwrap(),
    )
    .unwrap();
    let receipt = ["Quittung Zahnarztpraxis", "Behandlung Prophylaxe"];
    fs::write(
        root.join("IMG_2041.png"),
        common::picture(&receipt, "-png").unwrap(),
    )
    .unwrap();
    let before = fs::read(root.join("scan_0001.pdf")).unwrap();

    let mut index = Index::open(&elsewhere.path().join("index.sqlite")).unwrap();
    let report = index.scan(&root, |_| {}).unwrap();
    assert_eq!(report.texts, 2, "{report:?}");
    assert_eq!(report.awaiting_ocr, 0);

    let found = |words: &[&str]| -> Vec<String> {
        index
            .find(words)
            .unwrap()
            .into_iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    };
    assert_eq!(found(&["fernwaerme"]), ["scan_0001.pdf"]);
    assert_eq!(found(&["abrechnung", "guthaben"]), ["scan_0001.pdf"]);
    assert_eq!(found(&["prophylaxe"]), ["IMG_2041.png"]);

    let hash = hash_file(&root.join("scan_0001.pdf")).unwrap();
    match index.text(&hash).unwrap() {
        DocumentText::Read { reader, .. } => assert_eq!(reader, "ocr"),
        other => panic!("{other:?}"),
    }
    assert_eq!(fs::read(root.join("scan_0001.pdf")).unwrap(), before);
}
