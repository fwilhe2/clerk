// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Small things every page uses: how paths, times and counts are written, and file icons.

use std::path::Path;

use clerk_core::Report;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

pub fn load_css() {
    let css = gtk::CssProvider::new();
    css.load_from_string(
        ".result-context { opacity: 0.75; }
         .doc-text { font-family: monospace; font-size: 0.9em; background: none; }
         .doc-text text { background: none; }",
    );
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// A count with thousands separated, as people read it.
pub fn count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A path with the home folder written `~`.
pub fn tilde(path: &Path) -> String {
    let home = glib::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Seconds since 1970 as local time, like "7 Nov 2026, 19:32".
pub fn local_time(at: i64) -> String {
    glib::DateTime::from_unix_local(at)
        .and_then(|t| t.format("%e %b %Y, %H:%M"))
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| clerk_core::format_utc(at))
}

/// The icon of the kind of file `path` names, guessed from its name.
pub fn file_icon(path: &Path) -> gio::Icon {
    let (kind, _) = gio::content_type_guess(Some(path), None);
    gio::content_type_get_icon(&kind)
}

pub fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// What a scan found, in one line.
pub fn report_line(report: &Report) -> String {
    let mut parts = Vec::new();
    for (n, what) in [
        (report.added.len(), "added"),
        (report.moved.len(), "moved"),
        (report.copied.len(), "copied"),
        (report.changed.len(), "changed"),
        (report.removed.len(), "removed"),
    ] {
        if n > 0 {
            parts.push(format!("{} {what}", count(n)));
        }
    }
    format!("Scanned {}: {}", name_of(&report.root), parts.join(", "))
}

/// A snippet from the index, where `[` and `]` mark the words found, as Pango markup in bold.
///
/// ponytail: a bracket that was in the text itself reads as a mark too; marks that do not pair
/// up are shown as they are.
pub fn context_markup(context: &str) -> String {
    let mut out = String::new();
    let mut open = false;
    for (i, c) in context.char_indices() {
        match c {
            '[' if !open && context[i..].contains(']') => {
                out.push_str("<b>");
                open = true;
            }
            ']' if open => {
                out.push_str("</b>");
                open = false;
            }
            c => out.push_str(&glib::markup_escape_text(c.encode_utf8(&mut [0; 4]))),
        }
    }
    if open {
        out.push_str("</b>");
    }
    out
}

/// A label for a page: wrapped, left-aligned, and plain text.
pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .build();
    for class in classes {
        label.add_css_class(class);
    }
    label
}

/// Open `path` in the app the person uses for its kind of file.
pub fn open(window: &impl IsA<gtk::Window>, path: &Path) {
    gtk::FileLauncher::new(Some(&gio::File::for_path(path))).launch(
        Some(window),
        gio::Cancellable::NONE,
        |_| {},
    );
}

/// Show the folder `path` is in, with it selected.
pub fn show_in_folder(window: &impl IsA<gtk::Window>, path: &Path) {
    gtk::FileLauncher::new(Some(&gio::File::for_path(path))).open_containing_folder(
        Some(window),
        gio::Cancellable::NONE,
        |_| {},
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_become_bold_and_the_rest_is_escaped() {
        assert_eq!(
            context_markup("…ein [Kontoauszug] <1> & mehr…"),
            "…ein <b>Kontoauszug</b> &lt;1&gt; &amp; mehr…"
        );
        assert_eq!(context_markup("a [b"), "a [b");
    }

    #[test]
    fn counts_read_in_thousands() {
        assert_eq!(count(7), "7");
        assert_eq!(count(1284), "1,284");
        assert_eq!(count(1234567), "1,234,567");
    }
}
