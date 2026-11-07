// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! M4 (`doc/plan.md`): a download is offered the folder where files like it already are, with
//! the reason (C6), learned from the folder as the person keeps it.

use std::fs;
use std::path::Path;

use clerk_core::{Advice, Index, Reason};

mod common;
use common::pdf;

fn write(root: &Path, name: &str, bytes: &[u8]) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn advice(index: &Index, inbox: &Path, name: &str) -> Advice {
    index
        .inbox(inbox)
        .unwrap()
        .into_iter()
        .find(|item| item.path.file_name().unwrap() == name)
        .unwrap_or_else(|| panic!("{name} is not in the inbox"))
        .advice
}

/// A documents folder kept the way a person keeps one, and a Downloads folder beside it.
fn setup() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    Index,
) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let docs = root.join("Documents");
    for (n, month) in [(1, "07"), (2, "08"), (3, "09"), (4, "10")] {
        let text = format!("Kontoauszug {n}");
        write(
            &docs,
            &format!("Finanzen/Bank A/Kontoauszug_{n}_2026-{month}.pdf"),
            &pdf(&[&text, "IBAN DE02120300000000202051"]),
        );
    }
    write(
        &docs,
        "Finanzen/Bank B/Kontoauszug_1_2026-10.pdf",
        &pdf(&["Bank B"]),
    );
    for (year, amount) in [(2023, 41), (2024, 87), (2025, 12)] {
        let line =
            format!("Jahresabrechnung {year}, Kundennummer 4711081599, Guthaben {amount} EUR");
        write(
            &docs,
            &format!("Wohnung/Strom/abrechnung-{year}.txt"),
            line.as_bytes(),
        );
    }
    write(
        &docs,
        "Fotos/Urlaub/IMG_1001.jpg",
        b"\xff\xd8\xff not a real picture",
    );
    write(
        &docs,
        "Fotos/Garten/IMG_2001.jpg",
        b"\xff\xd8\xff not a real one either",
    );
    let downloads = root.join("Downloads");
    fs::create_dir_all(&downloads).unwrap();
    let mut index = Index::open(&root.join("index.sqlite")).unwrap();
    index.scan(&docs, |_| {}).unwrap();
    (dir, docs, downloads, index)
}

#[test]
fn a_bank_statement_in_downloads_is_offered_its_folder_with_the_reason() {
    let (_dir, docs, downloads, index) = setup();
    write(
        &downloads,
        "Kontoauszug_5_2026-11.pdf",
        &pdf(&["Kontoauszug 5"]),
    );
    let advice = advice(&index, &downloads, "Kontoauszug_5_2026-11.pdf");
    assert_eq!(
        advice,
        Advice::Suggest {
            folder: docs.join("Finanzen/Bank A"),
            reason: Reason::Name {
                template: "Kontoauszug_<n>_<date>.pdf".into(),
                there: 4,
                of: 5,
            },
        }
    );
    let Advice::Suggest { reason, .. } = advice else {
        unreachable!()
    };
    assert_eq!(
        reason.to_string(),
        "4 of the 5 files named Kontoauszug_<n>_<date>.pdf are there"
    );
}

#[test]
fn a_file_named_by_no_habit_is_placed_by_a_word_in_it() {
    let (_dir, docs, downloads, index) = setup();
    write(
        &downloads,
        "document(3).txt",
        b"Abschlagsplan 2026 fuer Kundennummer 4711081599",
    );
    assert_eq!(
        advice(&index, &downloads, "document(3).txt"),
        Advice::Suggest {
            folder: docs.join("Wohnung/Strom"),
            reason: Reason::Word {
                word: "4711081599".into(),
                there: 3,
                of: 3,
            },
        }
    );
}

#[test]
fn a_file_already_filed_is_said_to_be_and_an_unclear_one_gets_nothing() {
    let (_dir, docs, downloads, index) = setup();
    fs::copy(
        docs.join("Wohnung/Strom/abrechnung-2024.txt"),
        downloads.join("abrechnung(1).txt"),
    )
    .unwrap();
    assert_eq!(
        advice(&index, &downloads, "abrechnung(1).txt"),
        Advice::Known(vec![docs.join("Wohnung/Strom/abrechnung-2024.txt")])
    );
    write(&downloads, "IMG_3001.jpg", b"\xff\xd8\xff a third");
    assert_eq!(
        advice(&index, &downloads, "IMG_3001.jpg"),
        Advice::Nothing,
        "pictures named IMG_<n> are in two folders alike"
    );
}
