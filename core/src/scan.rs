// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Scanning a directory into the index: every regular file hashed, unless its size and
//! modification time say it has not changed, and the difference to the last scan recorded as
//! what happened to each document. Hidden entries and symbolic links are left alone.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::params;

use crate::index::{Index, key};
use crate::{Error, EventKind, Hash, Result, read, time};

/// What a scan found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub root: PathBuf,
    /// Regular files seen.
    pub files: usize,
    /// Files read and hashed, because they were new or had changed.
    pub hashed: usize,
    pub added: Vec<PathBuf>,
    /// Where it was copied from, and where to.
    pub copied: Vec<(PathBuf, PathBuf)>,
    /// Where it was, and where it is now.
    pub moved: Vec<(PathBuf, PathBuf)>,
    pub changed: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    /// What could not be read, and why. A file once indexed and now unreadable is kept.
    pub unreadable: Vec<(PathBuf, String)>,
}

struct Seen {
    path: PathBuf,
    hash: Hash,
    size: u64,
    mtime: i64,
    before: Option<Hash>,
}

fn hidden(entry: &walkdir::DirEntry) -> bool {
    entry.depth() > 0 && entry.file_name().to_string_lossy().starts_with('.')
}

impl Index {
    /// Scan `root` and remember it. `progress` is told how many files have been seen so far,
    /// now and then.
    pub fn scan(&mut self, root: &Path, mut progress: impl FnMut(usize)) -> Result<Report> {
        let root = root
            .canonicalize()
            .map_err(|e| Error::Io(root.to_owned(), e))?;
        if !root.is_dir() {
            return Err(Error::NotADirectory(root));
        }
        let index_dir = self
            .file
            .parent()
            .map(|d| d.canonicalize().unwrap_or(d.into()));
        if index_dir.is_some_and(|dir| dir.starts_with(&root)) {
            return Err(Error::IndexInside {
                root,
                index: self.file.clone(),
            });
        }
        let started = time::now();
        let all = self.files()?;
        let mut before: HashMap<PathBuf, (Hash, u64, i64)> = HashMap::new();
        let mut known: HashSet<Hash> = HashSet::new();
        for (path, hash, size, mtime) in all {
            match path.starts_with(&root) {
                true => {
                    before.insert(path, (hash, size, mtime));
                }
                false => {
                    known.insert(hash);
                }
            }
        }

        let mut report = Report {
            root: root.clone(),
            ..Report::default()
        };
        let mut seen: Vec<Seen> = Vec::new();
        let walk = walkdir::WalkDir::new(&root)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|e| !hidden(e));
        for entry in walk {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    let path = e.path().unwrap_or(&root).to_owned();
                    report.unreadable.push((path, e.to_string()));
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            report.files += 1;
            if report.files.is_multiple_of(500) {
                progress(report.files);
            }
            let path = entry.into_path();
            let old = before.get(&path).copied();
            let stat = std::fs::symlink_metadata(&path).and_then(|m| {
                let mtime = time::nanos(m.modified()?);
                Ok((m.len(), mtime))
            });
            let (size, mtime) = match stat {
                Ok(stat) => stat,
                Err(e) => {
                    report.unreadable.push((path.clone(), e.to_string()));
                    if let Some((hash, size, mtime)) = old {
                        seen.push(Seen {
                            path,
                            hash,
                            size,
                            mtime,
                            before: Some(hash),
                        });
                    }
                    continue;
                }
            };
            let hash = match old {
                Some((hash, s, m)) if s == size && m == mtime => hash,
                _ => match read::hash_file(&path) {
                    Ok(hash) => {
                        report.hashed += 1;
                        hash
                    }
                    Err(e) => {
                        report.unreadable.push((path.clone(), e.to_string()));
                        if let Some((hash, size, mtime)) = old {
                            seen.push(Seen {
                                path,
                                hash,
                                size,
                                mtime,
                                before: Some(hash),
                            });
                        }
                        continue;
                    }
                },
            };
            seen.push(Seen {
                path,
                hash,
                size,
                mtime,
                before: old.map(|(h, ..)| h),
            });
        }
        progress(report.files);

        let still: HashSet<&Path> = seen.iter().map(|s| s.path.as_path()).collect();
        let mut gone: Vec<(PathBuf, Hash)> = before
            .iter()
            .filter(|(path, _)| !still.contains(path.as_path()))
            .map(|(path, (hash, ..))| (path.clone(), *hash))
            .collect();
        gone.sort();
        let vanished: Vec<PathBuf> = gone.iter().map(|(path, _)| path.clone()).collect();

        let mut events: Vec<(EventKind, Hash, PathBuf, Option<PathBuf>)> = Vec::new();
        for s in &seen {
            match s.before {
                Some(old) if old == s.hash => {
                    known.insert(s.hash);
                }
                Some(_) => {
                    events.push((EventKind::Changed, s.hash, s.path.clone(), None));
                    report.changed.push(s.path.clone());
                    known.insert(s.hash);
                }
                None => {}
            }
        }
        // First seen at a path. Paired with a path that is gone, it moved there; with one
        // still there, it is a copy.
        let mut copies_of: HashMap<Hash, PathBuf> = seen
            .iter()
            .filter(|s| s.before.is_some())
            .map(|s| (s.hash, s.path.clone()))
            .collect();
        for s in seen.iter().filter(|s| s.before.is_none()) {
            let same_name =
                |(p, h): &(PathBuf, Hash)| *h == s.hash && p.file_name() == s.path.file_name();
            let pair = gone
                .iter()
                .position(same_name)
                .or_else(|| gone.iter().position(|(_, h)| *h == s.hash));
            if let Some(at) = pair {
                let (from, _) = gone.remove(at);
                report.moved.push((from.clone(), s.path.clone()));
                events.push((EventKind::Moved, s.hash, s.path.clone(), Some(from)));
            } else if known.contains(&s.hash) || copies_of.contains_key(&s.hash) {
                let from = copies_of.get(&s.hash).cloned();
                report
                    .copied
                    .push((from.clone().unwrap_or_default(), s.path.clone()));
                events.push((EventKind::Copied, s.hash, s.path.clone(), from));
            } else {
                report.added.push(s.path.clone());
                events.push((EventKind::Added, s.hash, s.path.clone(), None));
            }
            known.insert(s.hash);
            copies_of.entry(s.hash).or_insert_with(|| s.path.clone());
        }
        for (path, hash) in &gone {
            report.removed.push(path.clone());
            events.push((EventKind::Removed, *hash, path.clone(), None));
        }

        let finished = time::now();
        let tx = self.conn.transaction()?;
        for s in &seen {
            tx.execute(
                "INSERT OR IGNORE INTO documents (hash, size, first_seen) VALUES (?1, ?2, ?3)",
                params![&s.hash.0[..], s.size as i64, finished],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO files (path, hash, size, mtime) VALUES (?1, ?2, ?3, ?4)",
                params![key(&s.path), &s.hash.0[..], s.size as i64, s.mtime],
            )?;
        }
        for path in &vanished {
            tx.execute("DELETE FROM files WHERE path = ?1", [key(path)])?;
        }
        for (kind, hash, path, from) in &events {
            Index::record(&tx, finished, *kind, hash, path, from.as_deref())?;
        }
        // A root inside another is covered by it, and one containing others replaces them.
        let roots: Vec<PathBuf> = {
            let mut stmt = tx.prepare("SELECT path FROM roots")?;
            let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .map(crate::index::path_of)
                .collect()
        };
        if !roots.iter().any(|r| root.starts_with(r)) {
            for inner in roots.iter().filter(|r| r.starts_with(&root)) {
                tx.execute("DELETE FROM roots WHERE path = ?1", [key(inner)])?;
            }
            tx.execute(
                "INSERT INTO roots (path, added) VALUES (?1, ?2)",
                params![key(&root), finished],
            )?;
        }
        tx.execute(
            "INSERT INTO scans (root, started, finished, files) VALUES (?1, ?2, ?3, ?4)",
            params![key(&root), started, finished, report.files as i64],
        )?;
        tx.commit()?;
        Ok(report)
    }
}
