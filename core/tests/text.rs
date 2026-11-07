// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! M2 (`doc/plan.md`): a document is found by a word inside it — a PDF's text layer, an ODF
//! document, a spreadsheet, plain text — and its text belongs to its content, not its path.

use std::fs;
use std::path::Path;

use clerk_core::{DocumentText, Index, hash_file};

mod common;
use common::{fodt, pdf};

fn write(root: &Path, name: &str, bytes: &[u8]) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn found(index: &Index, words: &[&str]) -> Vec<String> {
    index
        .find(words)
        .unwrap()
        .into_iter()
        .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn find_finds_a_word_inside_a_statement() {
    let docs = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let root = docs.path().canonicalize().unwrap();
    let mut index = Index::open(&elsewhere.path().join("index.sqlite")).unwrap();
    write(
        &root,
        "Downloads/scan_0042.pdf",
        &pdf(&["Kontoauszug Nr. 10", "Saldo 1234,56 EUR"]),
    );
    write(
        &root,
        "Briefe/Kuendigung.fodt",
        &fodt(&["Hiermit kündige ich den Mietvertrag."]),
    );
    write(
        &root,
        "Tabellen/budget.csv",
        b"Posten,Betrag\nMiete,900\nStrom,80\n",
    );
    write(
        &root,
        "notes.txt",
        "Die Kaution wurde überwiesen.\n".as_bytes(),
    );
    write(&root, "backup.bin", b"\x00\x01\x02 binary");

    let report = index.scan(&root, |_| {}).unwrap();
    assert_eq!(report.texts, 4, "{report:?}");
    assert!(report.no_text.is_empty(), "{report:?}");

    assert_eq!(found(&index, &["saldo"]), ["scan_0042.pdf"]);
    assert_eq!(
        found(&index, &["AUSZUG", "1234"]),
        ["scan_0042.pdf"],
        "inside a word, any case"
    );
    assert_eq!(found(&index, &["mietvertrag"]), ["Kuendigung.fodt"]);
    assert_eq!(found(&index, &["KÜNDIGE"]), ["Kuendigung.fodt"]);
    assert_eq!(found(&index, &["strom"]), ["budget.csv"]);
    assert_eq!(found(&index, &["kaution", "überwiesen"]), ["notes.txt"]);
    assert_eq!(
        found(&index, &["downloads", "saldo"]),
        ["scan_0042.pdf"],
        "one word in the path, one in the text"
    );
    assert!(
        found(&index, &["saldo", "mietvertrag"]).is_empty(),
        "every word, one document"
    );

    let hit = index.find(&["saldo"]).unwrap().remove(0);
    let context = hit.context.unwrap();
    assert!(context.contains("[Saldo]"), "{context}");
    let by_name = index.find(&["scan_0042"]).unwrap().remove(0);
    assert_eq!(by_name.context, None, "found by its path, nothing to show");

    let binary = hash_file(&root.join("backup.bin")).unwrap();
    assert_eq!(index.text(&binary).unwrap(), DocumentText::None);
    assert_eq!(index.status().unwrap().with_text, 4);
}

#[test]
fn text_belongs_to_the_document_not_the_path() {
    let docs = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let root = docs.path().canonicalize().unwrap();
    let mut index = Index::open(&elsewhere.path().join("index.sqlite")).unwrap();
    write(&root, "letter.txt", b"Mahnung zur Rechnung");
    index.scan(&root, |_| {}).unwrap();

    fs::create_dir_all(root.join("Archiv")).unwrap();
    fs::rename(root.join("letter.txt"), root.join("Archiv/letter.txt")).unwrap();
    let mut read = 0;
    index
        .scan(&root, |p| {
            if let clerk_core::Progress::Texts(_, of) = p {
                read = of;
            }
        })
        .unwrap();
    assert_eq!(read, 0, "a moved document is not read again");
    assert_eq!(found(&index, &["mahnung"]), ["letter.txt"]);

    write(&root, "Archiv/letter.txt", b"Zahlungserinnerung");
    index.scan(&root, |_| {}).unwrap();
    assert!(
        found(&index, &["mahnung"]).is_empty(),
        "the old text left with the old content"
    );
    assert_eq!(found(&index, &["zahlungserinnerung"]), ["letter.txt"]);
}

#[test]
fn a_document_that_will_not_read_is_said_so_and_not_retried() {
    let docs = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let root = docs.path().canonicalize().unwrap();
    let mut index = Index::open(&elsewhere.path().join("index.sqlite")).unwrap();
    clerk_core::quiet_reader_panics();
    write(&root, "broken.pdf", b"%PDF-1.7 and then nothing");

    let first = index.scan(&root, |_| {}).unwrap();
    assert_eq!(first.no_text.len(), 1, "{first:?}");
    let hash = hash_file(&root.join("broken.pdf")).unwrap();
    assert!(matches!(
        index.text(&hash).unwrap(),
        DocumentText::Failed { .. }
    ));

    let mut read = 0;
    let again = index
        .scan(&root, |p| {
            if let clerk_core::Progress::Texts(_, of) = p {
                read = of;
            }
        })
        .unwrap();
    assert_eq!(read, 0);
    assert_eq!(again.no_text.len(), 1, "still reported");
}
