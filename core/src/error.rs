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
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Index(e)
    }
}
