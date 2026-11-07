// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The index: one SQLite file outside every directory it indexes (C4). Paths are stored as the
//! system's bytes, so a name that is not UTF-8 survives.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::{Error, Hash, Result};

/// What [`Index::open`] migrates to. Each step is one entry of [`MIGRATIONS`].
const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE roots (
        path BLOB PRIMARY KEY,
        added INTEGER NOT NULL
    );
    CREATE TABLE documents (
        hash BLOB PRIMARY KEY,
        size INTEGER NOT NULL,
        first_seen INTEGER NOT NULL
    );
    CREATE TABLE files (
        path BLOB PRIMARY KEY,
        hash BLOB NOT NULL REFERENCES documents(hash),
        size INTEGER NOT NULL,
        mtime INTEGER NOT NULL
    );
    CREATE INDEX files_hash ON files(hash);
    CREATE TABLE events (
        id INTEGER PRIMARY KEY,
        at INTEGER NOT NULL,
        kind TEXT NOT NULL,
        hash BLOB NOT NULL,
        path BLOB NOT NULL,
        from_path BLOB
    );
    CREATE INDEX events_hash ON events(hash);
    CREATE TABLE scans (
        id INTEGER PRIMARY KEY,
        root BLOB NOT NULL,
        started INTEGER NOT NULL,
        finished INTEGER NOT NULL,
        files INTEGER NOT NULL
    );
"#,
    r#"
    CREATE TABLE texts (
        id INTEGER PRIMARY KEY,
        hash BLOB NOT NULL UNIQUE REFERENCES documents(hash),
        -- Which reader gave the text, or failed; NULL when none applies.
        reader TEXT,
        -- crate::text::READERS when it was read.
        readers INTEGER NOT NULL,
        failed TEXT
    );
    -- The text of texts row `rowid`. Trigrams, so a word is found inside a compound.
    CREATE VIRTUAL TABLE text_search USING fts5(
        body,
        tokenize = 'trigram remove_diacritics 1'
    );
"#,
    r#"
    -- A scan or a picture read when OCR is installed, and not before.
    ALTER TABLE texts ADD COLUMN awaiting_ocr INTEGER NOT NULL DEFAULT 0;
"#,
    r#"
    -- Every move clerk made, which `clerk undo` reverses.
    CREATE TABLE moves (
        id INTEGER PRIMARY KEY,
        at INTEGER NOT NULL,
        hash BLOB NOT NULL,
        from_path BLOB NOT NULL,
        to_path BLOB NOT NULL,
        -- Across file systems, by copying and removing the original.
        copied INTEGER NOT NULL,
        undone INTEGER
    );
"#,
];

pub struct Index {
    pub(crate) conn: Connection,
    /// Where the index lives, which no scanned directory may contain.
    pub(crate) file: PathBuf,
}

/// A document found by [`Index::find`], at one of its paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub hash: Hash,
    pub size: u64,
    /// Where in its text a word was found, `[marked]`, when one was found there rather than in
    /// the path.
    pub context: Option<String>,
}

/// What the index holds of a document's text, from [`Index::text`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentText {
    /// Not read yet: it was not seen by a scan, or a scan stopped first.
    Unread,
    /// No reader applies to it.
    None,
    /// A scan or a picture, to be read once OCR is installed.
    AwaitingOcr,
    Read {
        reader: String,
        text: String,
    },
    Failed {
        reader: String,
        why: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// Seen for the first time.
    Added,
    /// The same content at another path, the first one still there.
    Copied,
    /// The same content at another path, the first one gone.
    Moved,
    /// New content at a path that held another.
    Changed,
    /// Gone from every path in the directory scanned.
    Removed,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Added => "added",
            EventKind::Copied => "copied",
            EventKind::Moved => "moved",
            EventKind::Changed => "changed",
            EventKind::Removed => "removed",
        }
    }

    fn parse(text: &str) -> Option<EventKind> {
        [
            EventKind::Added,
            EventKind::Copied,
            EventKind::Moved,
            EventKind::Changed,
            EventKind::Removed,
        ]
        .into_iter()
        .find(|k| k.as_str() == text)
    }
}

/// Something that happened to a document, as scans saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub at: i64,
    pub kind: EventKind,
    pub path: PathBuf,
    /// Where it was before, for a move or a copy.
    pub from: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootStatus {
    pub path: PathBuf,
    pub files: usize,
    /// When it was last scanned, if it was.
    pub scanned: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub index: PathBuf,
    pub roots: Vec<RootStatus>,
    pub files: usize,
    pub documents: usize,
    /// Documents in a folder whose text was read and can be searched.
    pub with_text: usize,
    /// Scans and pictures in a folder that OCR would read, were it installed.
    pub awaiting_ocr: usize,
}

#[cfg(unix)]
pub(crate) fn key(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
pub(crate) fn path_of(key: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(key).into()
}

// ponytail: lossy outside Unix; Windows wants its UTF-16 encoded once there is a Windows build.
#[cfg(not(unix))]
pub(crate) fn key(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

#[cfg(not(unix))]
pub(crate) fn path_of(key: Vec<u8>) -> PathBuf {
    String::from_utf8_lossy(&key).into_owned().into()
}

pub(crate) fn hash_of(bytes: Vec<u8>) -> rusqlite::Result<Hash> {
    <[u8; 32]>::try_from(bytes.as_slice())
        .map(Hash)
        .map_err(|_| {
            rusqlite::Error::InvalidColumnType(0, "hash".into(), rusqlite::types::Type::Blob)
        })
}

impl Index {
    /// `$XDG_DATA_HOME/clerk/index.sqlite`, or `~/.local/share/clerk/index.sqlite`.
    pub fn default_path() -> Option<PathBuf> {
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|home| PathBuf::from(home).join(".local/share"))
            })?;
        Some(data.join("clerk").join("index.sqlite"))
    }

    /// Open the index at `file`, creating it and its directory if need be, and bring its format
    /// up to date.
    pub fn open(file: &Path) -> Result<Index> {
        if let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| Error::Io(dir.to_owned(), e))?;
        }
        let conn = Connection::open(file)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > MIGRATIONS.len() as i64 {
            return Err(Error::NewerIndex(version));
        }
        for (step, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", step as i64 + 1)?;
            tx.commit()?;
        }
        let file = std::path::absolute(file).map_err(|e| Error::Io(file.to_owned(), e))?;
        Ok(Index { conn, file })
    }

    /// The directories scanned so far, which a scan with none named rescans.
    pub fn roots(&self) -> Result<Vec<PathBuf>> {
        let mut stmt = self.conn.prepare("SELECT path FROM roots ORDER BY path")?;
        let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
        Ok(rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(path_of)
            .collect())
    }

    /// Every file and its hash, size and modification time.
    pub(crate) fn files(&self) -> Result<Vec<(PathBuf, Hash, u64, i64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, hash, size, mtime FROM files")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                path_of(r.get(0)?),
                hash_of(r.get(1)?)?,
                r.get::<_, i64>(2)? as u64,
                r.get(3)?,
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The files where every one of `words` is found, ignoring case: in the path below the
    /// directory it was scanned from, or in the document's text. A word in the text is matched
    /// inside longer words too, as in the path.
    pub fn find(&self, words: &[&str]) -> Result<Vec<Found>> {
        let roots = self.roots()?;
        let words: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
        let in_text: Vec<HashMap<Hash, String>> = words
            .iter()
            .map(|w| self.text_matches(w))
            .collect::<Result<_>>()?;
        let mut found: Vec<Found> = self
            .files()?
            .into_iter()
            .filter_map(|(path, hash, size, _)| {
                let below = roots
                    .iter()
                    .filter(|root| path.starts_with(root))
                    .max_by_key(|root| root.as_os_str().len())
                    .and_then(|root| path.strip_prefix(root).ok())
                    .unwrap_or(&path);
                let name = below.to_string_lossy().to_lowercase();
                let mut context = None;
                for (word, text) in words.iter().zip(&in_text) {
                    if name.contains(word.as_str()) {
                        continue;
                    }
                    context.get_or_insert(text.get(&hash)?.clone());
                }
                Some(Found {
                    path,
                    hash,
                    size,
                    context,
                })
            })
            .collect();
        found.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(found)
    }

    /// The documents whose text contains `word`, each with where; a token is a trigram, so the
    /// 64 a snippet allows are about as many characters. Trigrams need three characters, so a
    /// shorter word is looked for in paths only.
    fn text_matches(&self, word: &str) -> Result<HashMap<Hash, String>> {
        if word.chars().count() < 3 {
            return Ok(HashMap::new());
        }
        let query = format!("\"{}\"", word.replace('"', "\"\""));
        let mut stmt = self.conn.prepare(
            "SELECT t.hash, snippet(text_search, 0, '[', ']', '…', 64) \
             FROM text_search JOIN texts t ON t.id = text_search.rowid \
             WHERE text_search MATCH ?1",
        )?;
        let rows = stmt.query_map([query], |r| {
            let context: String = r.get(1)?;
            Ok((hash_of(r.get(0)?)?, one_line(&context)))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// What the index holds of the text of the document `hash`.
    pub fn text(&self, hash: &Hash) -> Result<DocumentText> {
        let row = self
            .conn
            .query_row(
                "SELECT t.reader, t.failed, s.body, t.awaiting_ocr FROM texts t \
                 LEFT JOIN text_search s ON s.rowid = t.id WHERE t.hash = ?1",
                [&hash.0[..]],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?;
        Ok(match row {
            None => DocumentText::Unread,
            Some((.., true)) => DocumentText::AwaitingOcr,
            Some((None, ..)) => DocumentText::None,
            Some((Some(reader), Some(why), ..)) => DocumentText::Failed { reader, why },
            Some((Some(reader), None, text, _)) => DocumentText::Read {
                reader,
                text: text.unwrap_or_default(),
            },
        })
    }

    /// Every path the document `hash` is at now.
    pub fn paths_of(&self, hash: &Hash) -> Result<Vec<PathBuf>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM files WHERE hash = ?1 ORDER BY path")?;
        let rows = stmt.query_map([&hash.0[..]], |r| r.get::<_, Vec<u8>>(0))?;
        Ok(rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(path_of)
            .collect())
    }

    /// What scans saw happen to the document `hash`, oldest first.
    pub fn history(&self, hash: &Hash) -> Result<Vec<Event>> {
        let mut stmt = self
            .conn
            .prepare("SELECT at, kind, path, from_path FROM events WHERE hash = ?1 ORDER BY id")?;
        let rows = stmt.query_map([&hash.0[..]], |r| {
            let kind: String = r.get(1)?;
            Ok(Event {
                at: r.get(0)?,
                kind: EventKind::parse(&kind).unwrap_or(EventKind::Changed),
                path: path_of(r.get(2)?),
                from: r.get::<_, Option<Vec<u8>>>(3)?.map(path_of),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn status(&self) -> Result<Status> {
        let files = self.files()?;
        let mut roots = Vec::new();
        for path in self.roots()? {
            let scanned = self
                .conn
                .query_row(
                    "SELECT max(finished) FROM scans WHERE root = ?1",
                    [key(&path)],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten();
            roots.push(RootStatus {
                files: files.iter().filter(|(p, ..)| p.starts_with(&path)).count(),
                path,
                scanned,
            });
        }
        let documents: i64 =
            self.conn
                .query_row("SELECT count(DISTINCT hash) FROM files", [], |r| r.get(0))?;
        let with_text: i64 = self.conn.query_row(
            "SELECT count(*) FROM texts t JOIN text_search s ON s.rowid = t.id \
             WHERE t.hash IN (SELECT hash FROM files)",
            [],
            |r| r.get(0),
        )?;
        let awaiting_ocr: i64 = self.conn.query_row(
            "SELECT count(*) FROM texts WHERE awaiting_ocr AND hash IN (SELECT hash FROM files)",
            [],
            |r| r.get(0),
        )?;
        Ok(Status {
            index: self.file.clone(),
            roots,
            files: files.len(),
            documents: documents as usize,
            with_text: with_text as usize,
            awaiting_ocr: awaiting_ocr as usize,
        })
    }

    pub(crate) fn record(
        tx: &rusqlite::Transaction<'_>,
        at: i64,
        kind: EventKind,
        hash: &Hash,
        path: &Path,
        from: Option<&Path>,
    ) -> Result<()> {
        tx.execute(
            "INSERT INTO events (at, kind, hash, path, from_path) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![at, kind.as_str(), &hash.0[..], key(path), from.map(key)],
        )?;
        Ok(())
    }
}

/// A snippet on one line: the text's own line breaks and runs of spaces as single spaces.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
