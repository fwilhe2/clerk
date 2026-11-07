// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading a scan: the system's `tesseract`, with a PDF's pages drawn by poppler's `pdftoppm`
//! first. Both are optional (C3); without them a scan waits, and is read on the first scan after
//! they are installed. The text goes into the index only, never into the file. The original
//! file is opened by clerk alone: a picture reaches `tesseract` on its standard input, and a PDF
//! is copied into a directory of clerk's own for `pdftoppm`.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// ponytail: pages past this are not read; a long scanned book waits for a page-wise reader.
const MAX_PAGES: u32 = 50;

/// How long one program may run on one page before it is stopped.
const TIMEOUT: Duration = Duration::from_secs(120);

/// The OCR this system has.
#[derive(Clone, Debug)]
pub(crate) struct Ocr {
    /// Every language installed, as `tesseract -l` takes them: `deu+eng`.
    languages: String,
    /// Whether `pdftoppm` is there to draw a PDF's pages.
    pdf: bool,
}

/// Whether bytes are a picture `tesseract` reads: JPEG, PNG or TIFF.
pub(crate) fn is_picture(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\xff\xd8\xff")
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"II*\0")
        || bytes.starts_with(b"MM\0*")
}

/// Whether a PDF's text layer is too thin to be its text: a scan, with at most a page number or
/// a stamp on it.
pub(crate) fn is_scan(text: &str) -> bool {
    text.chars().filter(|c| c.is_alphanumeric()).count() < 32
}

impl Ocr {
    /// The OCR installed, or `None` without `tesseract` or any language for it.
    pub(crate) fn find() -> Option<Ocr> {
        let out = run(Command::new("tesseract").arg("--list-langs"), None).ok()?;
        // ponytail: every installed language at once; tesseract slows with each.
        let languages: Vec<&str> = std::str::from_utf8(&out)
            .ok()?
            .lines()
            .skip(1)
            .map(str::trim)
            .filter(|l| !l.is_empty() && !["osd", "equ", "snum"].contains(l))
            .collect();
        if languages.is_empty() {
            return None;
        }
        let pdf = run(Command::new("pdftoppm").arg("-v"), None).is_ok();
        Some(Ocr {
            languages: languages.join("+"),
            pdf,
        })
    }

    /// Whether this can read a scanned PDF, not only a picture.
    pub(crate) fn reads_pdf(&self) -> bool {
        self.pdf
    }

    /// The text of a picture.
    pub(crate) fn picture(&self, bytes: &[u8]) -> Result<String, String> {
        let out = run(
            Command::new("tesseract")
                .args(["stdin", "stdout", "-l", &self.languages])
                // The scan reads on every core already.
                .env("OMP_THREAD_LIMIT", "1"),
            Some(bytes),
        )?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// The text of a scanned PDF, page by page.
    pub(crate) fn pdf(&self, bytes: &[u8]) -> Result<String, String> {
        let dir = tempfile::Builder::new()
            .prefix("clerk-ocr-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let copy = dir.path().join("scan.pdf");
        std::fs::write(&copy, bytes).map_err(|e| e.to_string())?;
        run(
            Command::new("pdftoppm")
                .args(["-r", "300", "-gray", "-png", "-l", &MAX_PAGES.to_string()])
                .arg(&copy)
                .arg(dir.path().join("page")),
            None,
        )?;
        let mut pages: Vec<_> = std::fs::read_dir(dir.path())
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "png"))
            .collect();
        // pdftoppm pads page numbers to one width, so their names sort as the pages do.
        pages.sort();
        let mut text = String::new();
        for page in pages {
            let bytes = std::fs::read(&page).map_err(|e| e.to_string())?;
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&self.picture(&bytes)?);
        }
        Ok(text)
    }
}

/// Run `command`, writing `input` to it, and hand back what it wrote: an error when it is
/// missing, fails or takes longer than [`TIMEOUT`].
fn run(command: &mut Command, input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let name = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{name}: {e}"))?;
    let stdin = child.stdin.take();
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    std::thread::scope(|scope| {
        if let (Some(mut stdin), Some(input)) = (stdin, input) {
            // A program that stops reading early closes the pipe; its exit says why.
            scope.spawn(move || stdin.write_all(input));
        }
        let out = scope.spawn(move || {
            let mut out = Vec::new();
            stdout.read_to_end(&mut out).map(|_| out)
        });
        let err = scope.spawn(move || {
            let mut err = String::new();
            let _ = stderr.read_to_string(&mut err);
            err
        });
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!("{name} took longer than {}s", TIMEOUT.as_secs()));
                }
                Err(e) => break Err(format!("{name}: {e}")),
            }
        };
        let out = out.join().ok().and_then(Result::ok).unwrap_or_default();
        let err = err.join().unwrap_or_default();
        match status? {
            status if status.success() => Ok(out),
            status => Err(format!(
                "{name} {status}: {}",
                err.lines().last().unwrap_or_default()
            )),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_known_by_their_bytes() {
        assert!(is_picture(b"\xff\xd8\xff\xe0\x00\x10JFIF"));
        assert!(is_picture(b"\x89PNG\r\n\x1a\n...."));
        assert!(!is_picture(b"%PDF-1.4"));
    }

    #[test]
    fn a_page_number_is_not_a_text_layer() {
        assert!(is_scan("  \n 1 \n\n 2 "));
        assert!(!is_scan(
            "Kontoauszug Nr. 10 vom 31.10.2026, Saldo 1234,56 EUR"
        ));
    }

    #[test]
    fn a_missing_program_is_an_error_not_a_panic() {
        let missing = run(&mut Command::new("clerk-no-such-program"), None);
        assert!(missing.is_err());
    }
}
