// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Documents made for tests: PDFs with a text layer and without one, pictures of a page, flat
//! ODF. Shared by the core's and the CLI's tests, each using some.
#![allow(dead_code)]

use std::process::Command;

/// A PDF of `objects`, numbered from 1, the first the catalogue, with a correct
/// cross-reference table.
fn pdf_of(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn stream(dict: &str, bytes: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", bytes.len()).into_bytes();
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\nendstream");
    out
}

/// A one-page A4 PDF whose text layer holds `lines`, large enough to read in a picture of it.
pub fn pdf(lines: &[&str]) -> Vec<u8> {
    let mut content = String::from("BT /F1 20 Tf 72 720 Td 28 TL\n");
    for line in lines {
        content.push_str(&format!("({line}) Tj T*\n"));
    }
    content.push_str("ET");
    pdf_of(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
          /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
        stream("", content.as_bytes()),
    ])
}

/// A page's width and height in pixels, at 150 dpi.
const PIXELS: (u32, u32) = (1240, 1754);

/// A one-page A4 PDF that is only a picture of `lines`, as a scanner makes one: no text layer.
/// `None` without `pdftoppm` to draw the picture.
pub fn scanned_pdf(lines: &[&str]) -> Option<Vec<u8>> {
    // pdftoppm writes a JPEG in colour, -gray or not.
    let jpeg = picture(lines, "-jpeg")?;
    let (w, h) = PIXELS;
    let content = b"q 595 0 0 842 0 0 cm /Im1 Do Q";
    Some(pdf_of(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
          /Resources << /XObject << /Im1 4 0 R >> >> /Contents 5 0 R >>"
            .to_vec(),
        stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} \
                 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode"
            ),
            &jpeg,
        ),
        stream("", content),
    ]))
}

/// A picture of a page holding `lines`, as `pdftoppm` writes it with `format` (`-png`,
/// `-jpeg`). `None` without `pdftoppm`.
pub fn picture(lines: &[&str], format: &str) -> Option<Vec<u8>> {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("page.pdf");
    std::fs::write(&source, pdf(lines)).unwrap();
    let (w, h) = PIXELS;
    let status = Command::new("pdftoppm")
        .args([format, "-gray", "-singlefile"])
        .args(["-scale-to-x", &w.to_string(), "-scale-to-y", &h.to_string()])
        .arg(&source)
        .arg(dir.path().join("page"))
        .status()
        .ok()?;
    assert!(status.success());
    let out = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .find(|e| e.file_name() != "page.pdf")?;
    Some(std::fs::read(out.path()).unwrap())
}

/// Whether `tesseract` and `pdftoppm` are installed; a test that needs them says so and passes
/// without them.
pub fn ocr_installed() -> bool {
    let has = |program: &str, arg: &str| {
        Command::new(program)
            .arg(arg)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let found = has("tesseract", "--version") && has("pdftoppm", "-v");
    if !found {
        eprintln!("skipped: tesseract or pdftoppm is not installed");
    }
    found
}

/// A flat ODF text document, one paragraph per line.
pub fn fodt(lines: &[&str]) -> Vec<u8> {
    let body: String = lines
        .iter()
        .map(|l| format!("<text:p>{l}</text:p>"))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text">
<office:body><office:text>{body}</office:text></office:body>
</office:document>
"#
    )
    .into_bytes()
}
