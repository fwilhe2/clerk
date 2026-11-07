// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum Error {
    Index(rusqlite::Error),
    Io(PathBuf, std::io::Error),
    NotADirectory(PathBuf),
    /// The index file would be inside the directory it indexes, which would make clerk write
    /// into it (C1).
    IndexInside {
        root: PathBuf,
        index: PathBuf,
    },
    /// An index written by a newer clerk.
    NewerIndex(i64),
    NotAFile(PathBuf),
    /// A move would take a name in use; clerk never replaces a file (C5).
    Exists(PathBuf),
    /// A move would cross to another file system, which it does only when told to.
    AcrossFileSystems {
        from: PathBuf,
        to: PathBuf,
    },
    /// The file is not what clerk moved: it changed since, or did not copy true.
    Changed(PathBuf),
    NothingToUndo,
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Index(e) => write!(f, "the index: {e}"),
            Error::Io(path, e) => write!(f, "{}: {e}", path.display()),
            Error::NotADirectory(path) => write!(f, "{} is not a directory", path.display()),
            Error::IndexInside { root, index } => write!(
                f,
                "the index {} is inside {}, and clerk does not write into a directory it \
                 indexes; choose another place with --index",
                index.display(),
                root.display()
            ),
            Error::NewerIndex(version) => write!(
                f,
                "the index was written by a newer clerk (format {version}); update clerk"
            ),
            Error::NotAFile(path) => write!(f, "{} is not a file", path.display()),
            Error::Exists(path) => write!(
                f,
                "{} is already there, and clerk does not replace a file",
                path.display()
            ),
            Error::AcrossFileSystems { from, to } => write!(
                f,
                "{} and {} are on different file systems; moving means copying and removing \
                 the original, which clerk does with --across",
                from.display(),
                to.parent().unwrap_or(to).display()
            ),
            Error::Changed(path) => write!(
                f,
                "{} is not the file clerk moved: it changed since, so it is left alone",
                path.display()
            ),
            Error::NothingToUndo => write!(f, "there is no move to undo"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Index(e)
    }
}
