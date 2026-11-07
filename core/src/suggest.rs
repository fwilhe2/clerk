// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a new file belongs, learned from where the person already keeps files like it, and
//! said with the reason (C6): files named the same way, or documents sharing a word no other
//! folder's documents have. Learned afresh from the index each time; nothing is stored.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::index::{Index, hash_of};
use crate::ocr::Ocr;
use crate::text::Text;
use crate::{Hash, Result, read};

/// A folder must hold at least this many files named like the new one: a template is specific
/// enough that two make a habit.
const MIN_NAMED: usize = 2;

/// Or this many documents sharing a word with it; two sharing a word is often chance.
const MIN_SHARING: usize = 3;

/// And at least this share, in tenths, of all the files like it.
const MIN_SHARE: usize = 8;

/// ponytail: words of the new file's text looked up, longest first; a long letter's later words
/// are not.
const MAX_WORDS: usize = 300;

/// A word in more documents than this says nothing about where one belongs.
const MAX_HITS: usize = 2000;

/// Why a folder is suggested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// `there` of the `of` files named after `template` are in the folder.
    Name {
        template: String,
        there: usize,
        of: usize,
    },
    /// `there` of the `of` documents whose text contains `word` are in the folder.
    Word {
        word: String,
        there: usize,
        of: usize,
    },
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Name {
                template,
                there,
                of,
            } if there == of => {
                write!(f, "all {of} files named {template} are there")
            }
            Reason::Name {
                template,
                there,
                of,
            } => {
                write!(f, "{there} of the {of} files named {template} are there")
            }
            Reason::Word { word, there, of } if there == of => {
                write!(f, "all {of} documents containing “{word}” are there")
            }
            Reason::Word { word, there, of } => {
                write!(
                    f,
                    "{there} of the {of} documents containing “{word}” are there"
                )
            }
        }
    }
}

/// What clerk has to say about one new file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Advice {
    /// The same content is in the index already, at these paths.
    Known(Vec<PathBuf>),
    /// It belongs in `folder`, because of `reason`.
    Suggest { folder: PathBuf, reason: Reason },
    /// Nothing in the index says where.
    Nothing,
}

/// A file in an inbox, and what clerk has to say about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboxItem {
    pub path: PathBuf,
    pub advice: Advice,
}

/// A file name with what changes from one file to the next taken out: a date is `<date>`, any
/// other number `<n>`. `Kontoauszug_12_2026-11.pdf` is `Kontoauszug_<n>_<date>.pdf`.
pub fn template(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_digit()) {
            if let Some(len) = date(&chars[i..]) {
                out.push_str("<date>");
                i += len;
                continue;
            }
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            out.push_str("<n>");
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The length of a date at the start of `chars`: `2026-11-07`, `07.11.2026`, `20261107` or
/// `2026-11`, with any of `-_.` between, not followed by another digit.
fn date(chars: &[char]) -> Option<usize> {
    // `y`, `m` and `d` are digits of a year, month and day; `-` any separator.
    const SHAPES: [&str; 4] = ["yyyy-mm-dd", "dd-mm-yyyy", "yyyymmdd", "yyyy-mm"];
    SHAPES.iter().find_map(|shape| {
        let len = shape.len();
        let got = chars.get(..len)?;
        if chars.get(len).is_some_and(|c| c.is_ascii_digit()) {
            return None;
        }
        let mut parts: HashMap<char, u32> = HashMap::new();
        for (want, &c) in shape.chars().zip(got) {
            match want {
                '-' if "-_.".contains(c) => {}
                '-' => return None,
                _ => {
                    let digit = c.to_digit(10)?;
                    let part = parts.entry(want).or_default();
                    *part = *part * 10 + digit;
                }
            }
        }
        let valid = (1900..=2100).contains(&parts[&'y'])
            && (1..=12).contains(&parts[&'m'])
            && parts.get(&'d').is_none_or(|d| (1..=31).contains(d));
        valid.then_some(len)
    })
}

/// The folder holding most of `folders` — each a document's or a file's folder — if it holds
/// at least `min` and most of them, and how many.
fn best(folders: &[PathBuf], min: usize) -> Option<(PathBuf, usize)> {
    let mut counts: HashMap<&PathBuf, usize> = HashMap::new();
    for folder in folders {
        *counts.entry(folder).or_default() += 1;
    }
    let (folder, there) = counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))?;
    (there >= min && there * 10 >= folders.len() * MIN_SHARE).then(|| (folder.clone(), there))
}

/// The words of a text worth looking up: five characters or more, longest first, each once.
fn words(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 5)
        .map(str::to_lowercase)
        .filter(|w| seen.insert(w.clone()))
        .collect();
    words.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
    words.truncate(MAX_WORDS);
    words
}

impl Index {
    /// What clerk has to say about each file directly in `inbox`: where its content already is,
    /// or where it belongs and why. Reads the files, and changes nothing anywhere.
    pub fn inbox(&self, inbox: &Path) -> Result<Vec<InboxItem>> {
        let inbox = inbox
            .canonicalize()
            .map_err(|e| crate::Error::Io(inbox.to_owned(), e))?;
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&inbox)
            .map_err(|e| crate::Error::Io(inbox.clone(), e))?
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.path())
            .collect();
        paths.sort();
        self.advise(&paths)
    }

    /// What clerk has to say about each of `files`, as [`Index::inbox`] says it. The folders
    /// they are in are not learned from.
    pub fn advise(&self, files: &[PathBuf]) -> Result<Vec<InboxItem>> {
        let paths: Vec<PathBuf> = files
            .iter()
            .map(|f| f.canonicalize().map_err(|e| crate::Error::Io(f.clone(), e)))
            .collect::<Result<_>>()?;
        let inboxes: HashSet<&Path> = paths.iter().filter_map(|p| p.parent()).collect();

        // What is filed: every file outside the inboxes, by template, and every document's
        // folders.
        let mut by_template: HashMap<String, (String, Vec<PathBuf>)> = HashMap::new();
        let mut folders_of: HashMap<Hash, Vec<PathBuf>> = HashMap::new();
        for (path, hash, ..) in self.files()? {
            let Some(folder) = path.parent().filter(|f| !inboxes.contains(f)) else {
                continue;
            };
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let shown = template(&name);
            let entry = by_template
                .entry(shown.to_lowercase())
                .or_insert_with(|| (shown, Vec::new()));
            entry.1.push(folder.to_owned());
            let folders = folders_of.entry(hash).or_default();
            if !folders.iter().any(|f| f == folder) {
                folders.push(folder.to_owned());
            }
        }

        // By content and by name first; what that leaves is read for its text, on every core,
        // as a scan reads.
        let mut advice: Vec<Option<Advice>> = Vec::with_capacity(paths.len());
        let mut unread: Vec<(Hash, PathBuf)> = Vec::new();
        for path in &paths {
            let said = match read::hash_file(path) {
                Ok(hash) => {
                    let said = self.by_name(hash, path, &inboxes, &by_template)?;
                    if said.is_none() {
                        unread.push((hash, path.clone()));
                    }
                    said
                }
                Err(_) => Some(Advice::Nothing),
            };
            advice.push(said);
        }
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
        let texts: HashMap<PathBuf, Text> =
            crate::scan::read_all(unread, workers, Ocr::find().as_ref())
                .into_iter()
                .map(|(_, path, text)| (path, text))
                .collect();
        let mut items = Vec::with_capacity(paths.len());
        for (path, said) in paths.into_iter().zip(advice) {
            let advice = match said {
                Some(advice) => advice,
                None => match texts.get(&path) {
                    Some(Text::Read { text, .. }) => self.by_text(text, &folders_of)?,
                    _ => Advice::Nothing,
                },
            };
            items.push(InboxItem { path, advice });
        }
        Ok(items)
    }

    /// Where the file `path`, whose content is `hash`, is already, or where files named like it
    /// are; `None` when neither says.
    fn by_name(
        &self,
        hash: Hash,
        path: &Path,
        inboxes: &HashSet<&Path>,
        by_template: &HashMap<String, (String, Vec<PathBuf>)>,
    ) -> Result<Option<Advice>> {
        let known: Vec<PathBuf> = self
            .paths_of(&hash)?
            .into_iter()
            .filter(|p| !p.parent().is_some_and(|f| inboxes.contains(f)))
            .collect();
        if !known.is_empty() {
            return Ok(Some(Advice::Known(known)));
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if let Some((shown, folders)) = by_template.get(&template(&name).to_lowercase())
            && let Some((folder, there)) = best(folders, MIN_NAMED)
        {
            return Ok(Some(Advice::Suggest {
                folder,
                reason: Reason::Name {
                    template: shown.clone(),
                    there,
                    of: folders.len(),
                },
            }));
        }
        Ok(None)
    }

    /// Where the documents are that share a word of `text` no other folder's documents have.
    fn by_text(&self, text: &str, folders_of: &HashMap<Hash, Vec<PathBuf>>) -> Result<Advice> {
        let mut found: Option<(PathBuf, Reason)> = None;
        for word in words(text) {
            let hashes = self.documents_containing(&word)?;
            if hashes.len() > MAX_HITS {
                continue;
            }
            let folders: Vec<PathBuf> = hashes
                .iter()
                .filter_map(|h| folders_of.get(h))
                .flatten()
                .cloned()
                .collect();
            let Some((folder, there)) = best(&folders, MIN_SHARING) else {
                continue;
            };
            // More documents first; on a tie an identifier, a customer or account number, says
            // more than a word.
            let rank = |word: &str, there: usize| (there, word.chars().any(|c| c.is_ascii_digit()));
            let better = match &found {
                Some((
                    _,
                    Reason::Word {
                        word: was,
                        there: had,
                        ..
                    },
                )) => rank(&word, there) > rank(was, *had),
                _ => true,
            };
            if better {
                let of = folders.len();
                found = Some((folder, Reason::Word { word, there, of }));
            }
        }
        Ok(match found {
            Some((folder, reason)) => Advice::Suggest { folder, reason },
            None => Advice::Nothing,
        })
    }

    /// Every document whose text contains `word`.
    fn documents_containing(&self, word: &str) -> Result<Vec<Hash>> {
        let query = format!("\"{}\"", word.replace('"', "\"\""));
        let mut stmt = self.conn.prepare_cached(
            "SELECT t.hash FROM text_search JOIN texts t ON t.id = text_search.rowid \
             WHERE text_search MATCH ?1",
        )?;
        let rows = stmt.query_map([query], |r| hash_of(r.get(0)?))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_takes_out_dates_and_numbers() {
        assert_eq!(
            template("Kontoauszug_12_2026-11.pdf"),
            "Kontoauszug_<n>_<date>.pdf"
        );
        assert_eq!(template("Rechnung 07.11.2026.pdf"), "Rechnung <date>.pdf");
        assert_eq!(template("20261107_Scan.pdf"), "<date>_Scan.pdf");
        assert_eq!(template("Rechnung 2026-001.pdf"), "Rechnung <n>-<n>.pdf");
        assert_eq!(template("IMG_1234.jpg"), "IMG_<n>.jpg");
        assert_eq!(
            template("2026-13.pdf"),
            "<n>-<n>.pdf",
            "no thirteenth month"
        );
        assert_eq!(template("Brief.odt"), "Brief.odt");
    }

    #[test]
    fn a_folder_is_best_only_holding_most_of_them() {
        let a = PathBuf::from("/d/a");
        let b = PathBuf::from("/d/b");
        assert_eq!(
            best(&[a.clone(), a.clone(), a.clone(), b.clone()], 2),
            None,
            "three of four is not enough"
        );
        assert_eq!(
            best(&[a.clone(), a.clone(), a.clone(), a.clone(), b.clone()], 2),
            Some((a.clone(), 4))
        );
        assert_eq!(best(std::slice::from_ref(&a), 1), Some((a.clone(), 1)));
        assert_eq!(best(&[a.clone(), a.clone()], 3), None, "too few");
    }

    #[test]
    fn a_reason_says_how_many_of_how_many() {
        let reason = Reason::Name {
            template: "Kontoauszug_<n>_<date>.pdf".into(),
            there: 47,
            of: 47,
        };
        assert_eq!(
            reason.to_string(),
            "all 47 files named Kontoauszug_<n>_<date>.pdf are there"
        );
    }
}
