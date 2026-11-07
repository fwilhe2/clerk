// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading a document, which is all clerk ever does to one unasked (C1).

use std::fmt;
use std::fs::{File, OpenOptions};
use std::path::Path;

/// A document's identity: the BLAKE3 hash of its bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    /// The first twelve hex digits, enough to tell documents apart on screen.
    pub fn short(&self) -> String {
        self.to_string()[..12].to_owned()
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.short())
    }
}

/// `O_NOATIME`, whose value differs between architectures; only the ones checked here use it.
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
const O_NOATIME: i32 = 0o1_000_000;

/// Open `path` for reading, without updating its access time where the system allows that
/// (Linux, for files the person owns).
pub(crate) fn open(path: &Path) -> std::io::Result<File> {
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(file) = OpenOptions::new()
            .read(true)
            .custom_flags(O_NOATIME)
            .open(path)
        {
            return Ok(file);
        }
    }
    OpenOptions::new().read(true).open(path)
}

/// The hash of the file at `path`.
pub fn hash_file(path: &Path) -> std::io::Result<Hash> {
    let mut file = open(path)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&mut file)?;
    Ok(Hash(*hasher.finalize().as_bytes()))
}
