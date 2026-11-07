// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Moving a file, the one change clerk makes, and only when the person asks (C5): it never
//! takes a name that is in use, crosses to another file system only when told to, and goes into
//! a journal from which it can be undone.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use rusqlite::params;

use crate::index::{Index, hash_of, key, path_of};
use crate::{Error, EventKind, Hash, Result, read, time};

/// A move from the journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move {
    pub id: i64,
    pub at: i64,
    pub hash: Hash,
    pub from: PathBuf,
    pub to: PathBuf,
    /// Whether it went to another file system, by copying and then removing the original.
    pub copied: bool,
    /// When it was undone, if it was.
    pub undone: Option<i64>,
}

impl Index {
    /// Move `file` into `folder`, keeping its name. `across` allows a move to another file
    /// system, which copies the file and removes the original once the copy is checked.
    pub fn move_file(&mut self, file: &Path, folder: &Path, across: bool) -> Result<Move> {
        let from = file
            .canonicalize()
            .map_err(|e| Error::Io(file.to_owned(), e))?;
        let meta = std::fs::symlink_metadata(&from).map_err(|e| Error::Io(from.clone(), e))?;
        if !meta.is_file() {
            return Err(Error::NotAFile(from));
        }
        let folder = folder
            .canonicalize()
            .map_err(|e| Error::Io(folder.to_owned(), e))?;
        if !folder.is_dir() {
            return Err(Error::NotADirectory(folder));
        }
        let to = folder.join(
            from.file_name()
                .ok_or_else(|| Error::NotAFile(from.clone()))?,
        );
        if to == from {
            return Err(Error::Exists(to));
        }
        let hash = read::hash_file(&from).map_err(|e| Error::Io(from.clone(), e))?;
        let copied = place(&from, &to, &hash, across)?;
        let at = time::now();
        let tx = self.conn.transaction()?;
        Index::moved(&tx, at, &hash, &from, &to)?;
        tx.execute(
            "INSERT INTO moves (at, hash, from_path, to_path, copied) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![at, &hash.0[..], key(&from), key(&to), copied],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        self.read_text_at(&to)?;
        Ok(Move {
            id,
            at,
            hash,
            from,
            to,
            copied,
            undone: None,
        })
    }

    /// Undo the latest move not undone yet: the file goes back where it was, if it is still the
    /// same and nothing has taken its old place.
    pub fn undo(&mut self) -> Result<Move> {
        let mut last = self
            .journal()?
            .into_iter()
            .rev()
            .find(|m| m.undone.is_none())
            .ok_or(Error::NothingToUndo)?;
        match read::hash_file(&last.to) {
            Ok(hash) if hash == last.hash => {}
            Ok(_) => return Err(Error::Changed(last.to)),
            Err(e) => return Err(Error::Io(last.to, e)),
        }
        place(&last.to, &last.from, &last.hash, last.copied)?;
        let at = time::now();
        let tx = self.conn.transaction()?;
        Index::moved(&tx, at, &last.hash, &last.to, &last.from)?;
        tx.execute(
            "UPDATE moves SET undone = ?1 WHERE id = ?2",
            params![at, last.id],
        )?;
        tx.commit()?;
        self.read_text_at(&last.from)?;
        last.undone = Some(at);
        Ok(last)
    }

    /// Every move clerk made, oldest first.
    pub fn journal(&self) -> Result<Vec<Move>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, at, hash, from_path, to_path, copied, undone FROM moves ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Move {
                id: r.get(0)?,
                at: r.get(1)?,
                hash: hash_of(r.get(2)?)?,
                from: path_of(r.get(3)?),
                to: path_of(r.get(4)?),
                copied: r.get(5)?,
                undone: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Read the text of a file just moved to `path`, if it is in a scanned folder and its text
    /// was never read, so it is found by its text at once rather than after the next scan.
    fn read_text_at(&mut self, path: &Path) -> Result<()> {
        self.read_texts(path, &mut |_| {}, &mut crate::Report::default())
    }

    /// The index after `hash` went from `from` to `to`: the file is where it is now, as far as
    /// it is in a scanned folder, and its history says it moved.
    fn moved(
        tx: &rusqlite::Transaction<'_>,
        at: i64,
        hash: &Hash,
        from: &Path,
        to: &Path,
    ) -> Result<()> {
        let meta = std::fs::symlink_metadata(to).map_err(|e| Error::Io(to.to_owned(), e))?;
        let mtime = time::nanos(meta.modified().map_err(|e| Error::Io(to.to_owned(), e))?);
        tx.execute("DELETE FROM files WHERE path = ?1", [key(from)])?;
        let roots: Vec<PathBuf> = {
            let mut stmt = tx.prepare("SELECT path FROM roots")?;
            let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .map(path_of)
                .collect()
        };
        if roots.iter().any(|root| to.starts_with(root)) {
            tx.execute(
                "INSERT OR IGNORE INTO documents (hash, size, first_seen) VALUES (?1, ?2, ?3)",
                params![&hash.0[..], meta.len() as i64, at],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO files (path, hash, size, mtime) VALUES (?1, ?2, ?3, ?4)",
                params![key(to), &hash.0[..], meta.len() as i64, mtime],
            )?;
        }
        Index::record(tx, at, EventKind::Moved, hash, to, Some(from))
    }
}

/// Put the file at `from`, whose content is `hash`, at `to`, which must not exist. Whether it
/// was copied to another file system.
fn place(from: &Path, to: &Path, hash: &Hash, across: bool) -> Result<bool> {
    match rename_noreplace(from, to) {
        Ok(()) => Ok(false),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(Error::Exists(to.to_owned())),
        Err(e) if e.kind() == ErrorKind::CrossesDevices && across => {
            copy_across(from, to, hash).map(|()| true)
        }
        Err(e) if e.kind() == ErrorKind::CrossesDevices => Err(Error::AcrossFileSystems {
            from: from.to_owned(),
            to: to.to_owned(),
        }),
        Err(e) => Err(Error::Io(from.to_owned(), e)),
    }
}

/// Copy `from` to a new file `to`, check the copy is `hash`, keep the modification time, and
/// only then remove `from`. A copy that does not check out is removed again.
fn copy_across(from: &Path, to: &Path, hash: &Hash) -> Result<()> {
    let io = |path: &Path| {
        let path = path.to_owned();
        move |e| Error::Io(path, e)
    };
    let mut source = read::open(from).map_err(io(from))?;
    let modified = source
        .metadata()
        .and_then(|m| m.modified())
        .map_err(io(from))?;
    let mut target = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)
        .map_err(|e| match e.kind() {
            ErrorKind::AlreadyExists => Error::Exists(to.to_owned()),
            _ => Error::Io(to.to_owned(), e),
        })?;
    let written = std::io::copy(&mut source, &mut target)
        .and_then(|_| target.set_modified(modified))
        .and_then(|()| target.sync_all());
    drop(target);
    let checked = written.and_then(|()| read::hash_file(to));
    match checked {
        Ok(copy) if copy == *hash => {}
        Ok(_) => {
            let _ = std::fs::remove_file(to);
            return Err(Error::Changed(from.to_owned()));
        }
        Err(e) => {
            let _ = std::fs::remove_file(to);
            return Err(Error::Io(to.to_owned(), e));
        }
    }
    if let Err(e) = std::fs::remove_file(from) {
        let _ = std::fs::remove_file(to);
        return Err(Error::Io(from.to_owned(), e));
    }
    Ok(())
}

/// Rename without replacing: `renameat2(RENAME_NOREPLACE)` on Linux, and where that is not
/// there, a hard link and then removing the old name, which fails the same way on a name in use.
fn rename_noreplace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let c = |p: &Path| {
            CString::new(p.as_os_str().as_bytes())
                .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e))
        };
        let (old, new) = (c(from)?, c(to)?);
        // SAFETY: both are NUL-terminated paths that outlive the call.
        let done = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                old.as_ptr(),
                libc::AT_FDCWD,
                new.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if done == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if !matches!(e.raw_os_error(), Some(libc::EINVAL | libc::ENOSYS)) {
            return Err(e);
        }
    }
    std::fs::hard_link(from, to)?;
    std::fs::remove_file(from)
}
