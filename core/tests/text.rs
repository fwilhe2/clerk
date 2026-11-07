// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! M2 (`doc/plan.md`): a document is found by a word inside it — a PDF's text layer, an ODF
//! document, a spreadsheet, plain text — and its text belongs to its content, not its path.

use std::fs;
use std::path::Path;

use clerk_core::{DocumentText, Index, hash_file};

/// A one-page PDF whose text layer holds `lines`, with a correct cross-reference table.
fn pdf(lines: &[&str]) -> Vec<u8> {
    let mut content = String::from("BT /F1 12 Tf 72 720 Td 14 TL\n");
    for line in lines {
        content.push_str(&format!("({line}) Tj T*\n"));
    }
    content.push_str("ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
            .to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica \
         /Encoding /WinAnsiEncoding >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len() + 1
        ),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// A flat ODF text document, one paragraph per line.
fn fodt(lines: &[&str]) -> Vec<u8> {
    let body: String = lines
        .iter()
        .map(|l| format!("<text:p>{l}</text:p>"))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text">
<office:body><office:text>{body}</office:text></office:body>
</office:document>
"#
    )
    .into_bytes()
}

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
    write(&root, "photo.jpg", b"\xff\xd8\xff\xe0\x00\x10JFIF\x00");

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

    let photo = hash_file(&root.join("photo.jpg")).unwrap();
    assert_eq!(index.text(&photo).unwrap(), DocumentText::None);
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
