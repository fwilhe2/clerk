// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Find: documents by every word in their path or text, and one document's places, text and
//! history beside them.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use clerk_core::{Found, Hash};

use crate::{Ctx, document, ui};

/// More rows than this make the list slow to build, and are more than anyone reads.
const SHOWN: usize = 300;

pub struct Find {
    ctx: Rc<Ctx>,
    pub widget: gtk::Widget,
    pub split: adw::NavigationSplitView,
    entry: gtk::SearchEntry,
    list: gtk::ListBox,
    found: RefCell<Vec<Found>>,
    content: gtk::Stack,
    empty: adw::StatusPage,
    detail: gtk::ScrolledWindow,
    /// The document shown, to show again when the index changes.
    shown: RefCell<Option<(Hash, PathBuf)>>,
}

impl Find {
    pub fn new(ctx: &Rc<Ctx>) -> Rc<Find> {
        let entry = gtk::SearchEntry::builder()
            .placeholder_text("Search names and text")
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        let list = gtk::ListBox::new();
        list.add_css_class("navigation-sidebar");
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.append(&entry);
        sidebar.append(
            &gtk::ScrolledWindow::builder()
                .child(&list)
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build(),
        );

        let empty = adw::StatusPage::builder()
            .icon_name("system-search-symbolic")
            .build();
        let detail = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let content = gtk::Stack::new();
        content.add_named(&empty, Some("empty"));
        content.add_named(&detail, Some("document"));

        let split = adw::NavigationSplitView::builder()
            .sidebar(&adw::NavigationPage::new(&sidebar, "Find"))
            .content(&adw::NavigationPage::new(&content, "Document"))
            .min_sidebar_width(300.0)
            .max_sidebar_width(420.0)
            .sidebar_width_fraction(0.36)
            .build();

        let find = Rc::new(Find {
            ctx: ctx.clone(),
            widget: split.clone().upcast(),
            split,
            entry,
            list,
            found: RefCell::new(Vec::new()),
            content,
            empty,
            detail,
            shown: RefCell::new(None),
        });
        {
            let weak = Rc::downgrade(&find);
            find.entry.connect_search_changed(move |_| {
                if let Some(find) = weak.upgrade() {
                    find.search();
                }
            });
        }
        {
            let weak = Rc::downgrade(&find);
            find.entry.connect_activate(move |_| {
                if let Some(find) = weak.upgrade()
                    && let Some(row) = find.list.row_at_index(0)
                {
                    row.activate();
                }
            });
        }
        {
            let weak = Rc::downgrade(&find);
            find.list.connect_row_selected(move |_, row| {
                if let (Some(find), Some(row)) = (weak.upgrade(), row) {
                    find.show_row(row.index());
                }
            });
        }
        {
            let weak = Rc::downgrade(&find);
            find.list.connect_row_activated(move |_, row| {
                if let Some(find) = weak.upgrade() {
                    find.show_row(row.index());
                    find.split.set_show_content(true);
                }
            });
        }
        {
            let weak = Rc::downgrade(&find);
            ctx.on_changed(move || {
                if let Some(find) = weak.upgrade() {
                    find.refresh();
                }
            });
        }
        find.search();
        find
    }

    pub fn focus(&self) {
        self.entry.grab_focus();
    }

    fn words(&self) -> Vec<String> {
        self.entry
            .text()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }

    /// Search again, keeping the document shown when it is still found.
    fn refresh(&self) {
        let shown = self.shown.borrow().clone();
        self.search();
        if let Some((hash, path)) = shown {
            let paths = self.ctx.index.borrow().paths_of(&hash).unwrap_or_default();
            let path = if paths.contains(&path) {
                Some(path)
            } else {
                paths.into_iter().next()
            };
            if let Some(path) = path {
                self.show_document(&hash, &path, &self.words());
            }
        }
    }

    fn search(&self) {
        let words = self.words();
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        if words.is_empty() {
            self.found.borrow_mut().clear();
            self.show_empty();
            return;
        }
        let refs: Vec<&str> = words.iter().map(String::as_str).collect();
        let found = match self.ctx.index.borrow().find(&refs) {
            Ok(found) => found,
            Err(e) => {
                self.ctx.toast(&format!("Not searched: {e}"));
                Vec::new()
            }
        };
        let roots = self.ctx.index.borrow().roots().unwrap_or_default();
        for f in found.iter().take(SHOWN) {
            self.list.append(&row(f, &roots));
        }
        if found.len() > SHOWN {
            let more = ui::label(
                &format!(
                    "{} more. Add a word to find fewer.",
                    ui::count(found.len() - SHOWN)
                ),
                &["dim-label", "caption"],
            );
            more.set_margin_top(12);
            more.set_margin_bottom(12);
            more.set_xalign(0.5);
            let row = gtk::ListBoxRow::builder()
                .child(&more)
                .selectable(false)
                .activatable(false)
                .build();
            self.list.append(&row);
        }
        let none = found.is_empty();
        *self.found.borrow_mut() = found;
        if none {
            self.shown.replace(None);
            self.empty.set_title("Nothing Found");
            self.empty.set_description(Some(
                "No document has every one of these words in its name, its folder or its text. \
                 In the text, a word needs at least three letters.",
            ));
            self.content.set_visible_child_name("empty");
        } else if !self.split.is_collapsed()
            && let Some(first) = self.list.row_at_index(0)
        {
            self.list.select_row(Some(&first));
        }
    }

    fn show_empty(&self) {
        self.shown.replace(None);
        let documents = self
            .ctx
            .index
            .borrow()
            .status()
            .map(|s| s.documents)
            .unwrap_or(0);
        self.empty
            .set_title(&format!("Search {} Documents", ui::count(documents)));
        self.empty.set_description(Some(
            "Every word you type has to be in a document's name, its folder or its text. \
             Case doesn't matter, and part of a word is enough.\n\n\
             Drop a file on this window to see where its copy is.",
        ));
        self.content.set_visible_child_name("empty");
    }

    fn show_row(&self, i: i32) {
        let found = self.found.borrow().get(i as usize).cloned();
        if let Some(f) = found {
            self.show_document(&f.hash, &f.path, &self.words());
        }
    }

    /// Show the document with `hash`, at `path`, its text marked where `words` are.
    pub fn show_document(&self, hash: &Hash, path: &Path, words: &[String]) {
        self.shown.replace(Some((*hash, path.to_owned())));
        let page = document::page(
            &self.ctx,
            hash,
            path,
            words,
            &self.split,
            "Back to the Results",
        );
        self.detail.set_child(Some(&page));
        self.content.set_visible_child_name("document");
    }
}

fn row(found: &Found, roots: &[PathBuf]) -> gtk::ListBoxRow {
    let icon = gtk::Image::from_gicon(&ui::file_icon(&found.path));
    icon.set_pixel_size(32);
    icon.set_valign(gtk::Align::Start);
    let name = ui::label(&ui::name_of(&found.path), &["heading"]);
    let folder = ui::label(&folder_of(&found.path, roots), &["dim-label", "caption"]);
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 2);
    texts.append(&name);
    texts.append(&folder);
    if let Some(context) = &found.context {
        let snippet = ui::label("", &["caption", "result-context"]);
        snippet.set_markup(&ui::context_markup(context));
        snippet.set_lines(3);
        snippet.set_ellipsize(gtk::pango::EllipsizeMode::End);
        snippet.set_margin_top(2);
        texts.append(&snippet);
    }
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    line.set_margin_top(6);
    line.set_margin_bottom(6);
    line.append(&icon);
    line.append(&texts);
    gtk::ListBoxRow::builder().child(&line).build()
}

/// The folder a file is in, from the folder above the one it was scanned in.
fn folder_of(path: &Path, roots: &[PathBuf]) -> String {
    let parent = path.parent().unwrap_or(path);
    match roots.iter().find(|r| parent.starts_with(r)) {
        Some(root) => {
            let top = root.file_name().map(PathBuf::from).unwrap_or_default();
            top.join(parent.strip_prefix(root).unwrap_or(parent))
                .display()
                .to_string()
        }
        None => ui::tilde(parent),
    }
}
