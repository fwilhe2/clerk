// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a document says: its text, read once per document and kept in the index for search. A
//! PDF's text layer, anything grind opens (ODF, `.docx`, `.xlsx`, CSV, Markdown), and plain text.

use std::io::Read as _;
use std::path::Path;

use crate::read;

/// Raised whenever a reader gets better, so documents read by an older one are read again.
pub(crate) const READERS: i64 = 1;

/// The thread a reader runs on. Readers parse whatever is in the folder and may panic on it; see
/// [`quiet_reader_panics`].
const THREAD: &str = "clerk-reader";

/// ponytail: larger files are not read for text, which keeps one huge scan from holding a
/// scan's memory; a streaming reader would lift it.
const MAX_FILE: u64 = 256 << 20;

/// ponytail: text past this is not searched. A statement or a letter is far below it.
const MAX_TEXT: usize = 8 << 20;

/// What reading a document for its text gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Text {
    /// `reader` read `text`.
    Read { reader: &'static str, text: String },
    /// No reader applies: a picture, an archive, a file too large.
    None,
    /// `reader` applies and failed.
    Failed { reader: &'static str, why: String },
}

/// Read the text of the file at `path`.
pub(crate) fn read(path: &Path) -> std::io::Result<Text> {
    let mut bytes = Vec::new();
    read::open(path)?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE {
        return Ok(Text::None);
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(of(&name, &bytes))
}

fn of(name: &str, bytes: &[u8]) -> Text {
    let (reader, got): (&'static str, Option<Result<String, String>>) =
        if bytes.starts_with(b"%PDF-") {
            ("pdf", guarded(|| pdf(bytes)))
        } else {
            ("grind", guarded(|| grind_open::text(name, bytes)).flatten())
        };
    match got {
        Some(Ok(text)) => Text::Read {
            reader,
            text: capped(text),
        },
        Some(Err(why)) => Text::Failed { reader, why },
        None if reader == "pdf" => Text::Failed {
            reader,
            why: "the reader stopped on this file".to_owned(),
        },
        None => match plain(bytes) {
            Some(text) => Text::Read {
                reader: "plain",
                text: capped(text.to_owned()),
            },
            None => Text::None,
        },
    }
}

fn pdf(bytes: &[u8]) -> Result<String, String> {
    pdf_extract::extract_text_from_mem(bytes).map_err(|e| e.to_string())
}

/// Run `f` on a reader thread, `None` if it panicked.
fn guarded<T: Send>(f: impl FnOnce() -> T + Send) -> Option<T> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name(THREAD.to_owned())
            .spawn_scoped(scope, f)
            .ok()?
            .join()
            .ok()
    })
}

/// UTF-8 without a NUL byte in its first kilobytes is text; anything else is not read as text.
fn plain(bytes: &[u8]) -> Option<&str> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return None;
    }
    std::str::from_utf8(bytes).ok()
}

fn capped(mut text: String) -> String {
    if text.len() > MAX_TEXT {
        let mut end = MAX_TEXT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// Keep a reader's panic, which only means one file could not be read, off the terminal; every
/// other thread's panic is reported as before. For a shell to call once, at start.
pub fn quiet_reader_panics() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().name() != Some(THREAD) {
            before(info);
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_utf8_without_nul() {
        assert_eq!(
            of("a.txt", "Grüße\n".as_bytes()),
            Text::Read {
                reader: "plain",
                text: "Grüße\n".into()
            }
        );
        assert_eq!(of("a.bin", b"\x00\x01\x02"), Text::None);
        assert_eq!(of("a.txt", b"\xff\xfe latin"), Text::None);
    }

    #[test]
    fn a_broken_pdf_fails_without_a_panic_escaping() {
        quiet_reader_panics();
        match of("x.pdf", b"%PDF-1.7 nothing after") {
            Text::Failed { reader: "pdf", .. } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn text_is_capped_on_a_character_boundary() {
        let long = "ä".repeat(MAX_TEXT);
        let capped = capped(long);
        assert!(capped.len() <= MAX_TEXT && capped.len() > MAX_TEXT - 2);
    }
}
