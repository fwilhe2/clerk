// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! One document: where it is, its text, and its history. Find and Browse both show it.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use clerk_core::{DocumentText, EventKind, Hash};

use crate::{Ctx, ui};

/// The text shown of one document; the index keeps all of it.
const TEXT_SHOWN: usize = 200_000;

/// The page of the document with `hash`, at `path`, its text marked where `words` are. In a
/// collapsed `split` it starts with a button back to the sidebar, which `back` names.
pub fn page(
    ctx: &Rc<Ctx>,
    hash: &Hash,
    path: &Path,
    words: &[String],
    split: &adw::NavigationSplitView,
    back: &str,
) -> gtk::Widget {
    let index = ctx.index.borrow();
    let paths = index.paths_of(hash).unwrap_or_default();
    let history = index.history(hash).unwrap_or_default();
    let text = index.text(hash).unwrap_or(DocumentText::Unread);
    drop(index);

    let page = gtk::Box::new(gtk::Orientation::Vertical, 24);
    page.set_margin_top(24);
    page.set_margin_bottom(48);
    page.set_margin_start(16);
    page.set_margin_end(16);

    let back = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text(back)
        .halign(gtk::Align::Start)
        .build();
    back.add_css_class("flat");
    split
        .bind_property("collapsed", &back, "visible")
        .sync_create()
        .build();
    {
        let split = split.clone();
        back.connect_clicked(move |_| split.set_show_content(false));
    }
    page.append(&back);
    page.append(&head(ctx, path));
    page.append(&places(ctx, path, &paths));
    page.append(&text_group(&text, words));
    if !history.is_empty() {
        let group = adw::PreferencesGroup::builder().title("History").build();
        for event in history.iter().rev() {
            let (what, from) = match (event.kind, &event.from) {
                (EventKind::Added, _) => ("Added", None),
                (EventKind::Copied, from) => ("Copied", from.as_ref()),
                (EventKind::Moved, from) => ("Moved", from.as_ref()),
                (EventKind::Changed, _) => ("Changed", None),
                (EventKind::Removed, _) => ("Removed", None),
            };
            let mut subtitle = format!("{} · {}", what, ui::local_time(event.at));
            if let Some(from) = from {
                subtitle += &format!(" · from {}", ui::tilde(from));
            }
            let row = adw::ActionRow::builder()
                .title(ui::tilde(&event.path))
                .subtitle(subtitle)
                .use_markup(false)
                .build();
            row.add_css_class("property");
            group.add(&row);
        }
        page.append(&group);
    }
    adw::Clamp::builder()
        .maximum_size(720)
        .child(&page)
        .build()
        .upcast()
}

fn head(ctx: &Rc<Ctx>, path: &Path) -> gtk::Widget {
    let icon = gtk::Image::from_gicon(&ui::file_icon(path));
    icon.set_pixel_size(64);
    let title = ui::label(&ui::name_of(path), &["title-2"]);
    title.set_selectable(true);
    let folder = path.parent().map(ui::tilde).unwrap_or_default();
    let names = gtk::Box::new(gtk::Orientation::Vertical, 4);
    names.set_valign(gtk::Align::Center);
    names.append(&title);
    names.append(&ui::label(&folder, &["dim-label"]));
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    top.append(&icon);
    top.append(&names);

    let open = gtk::Button::builder()
        .label("Open")
        .css_classes(["pill", "suggested-action"])
        .build();
    let show = gtk::Button::builder()
        .label("Show in Folder")
        .css_classes(["pill"])
        .build();
    for (button, reveal) in [(&open, false), (&show, true)] {
        let window = ctx.window.clone();
        let path = path.to_owned();
        button.connect_clicked(move |_| match reveal {
            false => ui::open(&window, &path),
            true => ui::show_in_folder(&window, &path),
        });
    }
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.append(&open);
    buttons.append(&show);
    let head = gtk::Box::new(gtk::Orientation::Vertical, 18);
    head.append(&top);
    head.append(&buttons);
    head.upcast()
}

fn places(ctx: &Rc<Ctx>, shown: &Path, paths: &[PathBuf]) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder()
        .title("Where It Is")
        .build();
    match paths.len() {
        0 => group.set_description(Some("It is not in your documents any more.")),
        1 => {}
        n => group.set_description(Some(&format!(
            "The same content is in {n} places, so these are copies of one document."
        ))),
    }
    for path in paths {
        let row = adw::ActionRow::builder()
            .title(ui::name_of(path))
            .subtitle(path.parent().map(ui::tilde).unwrap_or_default())
            .use_markup(false)
            .build();
        if path == shown && paths.len() > 1 {
            row.add_prefix(&gtk::Image::from_icon_name("object-select-symbolic"));
        }
        let button = gtk::Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text("Show in Folder")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let window = ctx.window.clone();
        let path = path.clone();
        button.connect_clicked(move |_| ui::show_in_folder(&window, &path));
        row.add_suffix(&button);
        group.add(&row);
    }
    group.upcast()
}

fn text_group(text: &DocumentText, words: &[String]) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder().title("Text").build();
    let note = match text {
        DocumentText::Read { reader, text } => {
            group.set_description(Some(&format!(
                "Read by {reader}. This is what Find searches."
            )));
            group.add(&text_view(text, words));
            return group.upcast();
        }
        DocumentText::Unread => "Not read yet. clerk reads it on the next scan.".to_owned(),
        DocumentText::None => {
            "clerk doesn't read text from this kind of file. It is found by its name and folder."
                .to_owned()
        }
        DocumentText::AwaitingOcr => "This is a scan or a picture, so its text has to be \
             recognised. Install tesseract, and poppler's pdftoppm for a PDF, and clerk reads it \
             on the next scan."
            .to_owned(),
        DocumentText::Failed { reader, why } => {
            format!("Its text could not be read ({reader}): {why}. It is still found by its name.")
        }
    };
    let label = ui::label(&note, &[]);
    label.set_margin_top(12);
    label.set_margin_bottom(12);
    label.set_margin_start(12);
    label.set_margin_end(12);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("card");
    card.append(&label);
    group.add(&card);
    group.upcast()
}

fn text_view(text: &str, words: &[String]) -> gtk::Widget {
    let mut shown = text.trim_end();
    let mut cut = false;
    if shown.len() > TEXT_SHOWN {
        let mut end = TEXT_SHOWN;
        while !shown.is_char_boundary(end) {
            end -= 1;
        }
        shown = &shown[..end];
        cut = true;
    }
    let buffer = gtk::TextBuffer::new(None);
    buffer.set_text(shown);
    if cut {
        buffer.insert(
            &mut buffer.end_iter(),
            "\n\n… The rest of the text is in the index, and searched; it is not shown here.",
        );
    }
    let mark = buffer
        .create_tag(Some("found"), &[("weight", &700i32.to_value())])
        .expect("a new tag");
    mark.set_background_rgba(Some(&gtk::gdk::RGBA::new(0.96, 0.83, 0.18, 0.45)));
    let mut first = None;
    for word in words.iter().filter(|w| w.chars().count() >= 3) {
        let mut at = buffer.start_iter();
        while let Some((start, end)) =
            at.forward_search(word, gtk::TextSearchFlags::CASE_INSENSITIVE, None)
        {
            buffer.apply_tag(&mark, &start, &end);
            if first
                .as_ref()
                .is_none_or(|f: &gtk::TextIter| start.offset() < f.offset())
            {
                first = Some(start);
            }
            at = end;
        }
    }
    let view = gtk::TextView::builder()
        .buffer(&buffer)
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(12)
        .bottom_margin(12)
        .left_margin(14)
        .right_margin(14)
        .monospace(true)
        .build();
    view.add_css_class("doc-text");
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&view)
        .min_content_height(80)
        .max_content_height(420)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    if let Some(first) = first {
        let mark = buffer.create_mark(None, &first, true);
        // Scrolled once the view has a size, which it does not have yet.
        let view = view.clone();
        gtk::glib::idle_add_local_once(move || {
            view.scroll_to_mark(&mark, 0.1, true, 0.0, 0.2);
        });
    }
    let card = gtk::Frame::new(None);
    card.add_css_class("card");
    card.set_child(Some(&scrolled));
    card.upcast()
}
