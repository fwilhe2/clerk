// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The index: one SQLite file outside every directory it indexes (C4). Paths are stored as the
//! system's bytes, so a name that is not UTF-8 survives.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::{Error, Hash, Result};

/// What [`Index::open`] migrates to. Each step is one entry of [`MIGRATIONS`].
const MIGRATIONS: &[&str] = &[r#"
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
"#];

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

    /// The files whose path, below the directory it was scanned from, contains every one of
    /// `words`, ignoring case.
    pub fn find(&self, words: &[&str]) -> Result<Vec<Found>> {
        let roots = self.roots()?;
        let words: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
        let mut found: Vec<Found> = self
            .files()?
            .into_iter()
            .filter(|(path, ..)| {
                let below = roots
                    .iter()
                    .filter(|root| path.starts_with(root))
                    .max_by_key(|root| root.as_os_str().len())
                    .and_then(|root| path.strip_prefix(root).ok())
                    .unwrap_or(path);
                let text = below.to_string_lossy().to_lowercase();
                words.iter().all(|w| text.contains(w.as_str()))
            })
            .map(|(path, hash, size, _)| Found { path, hash, size })
            .collect();
        found.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(found)
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
        Ok(Status {
            index: self.file.clone(),
            roots,
            files: files.len(),
            documents: documents as usize,
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
