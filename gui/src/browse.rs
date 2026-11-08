// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Browse: the folders as they are on disk, from the ones scanned down, and a document beside
//! them as Find shows it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::{Ctx, document, ui};

enum Entry {
    Folder(PathBuf),
    File(PathBuf),
}

pub struct Browse {
    ctx: Rc<Ctx>,
    pub widget: gtk::Widget,
    pub split: adw::NavigationSplitView,
    up: gtk::Button,
    title: gtk::Label,
    subtitle: gtk::Label,
    list: gtk::ListBox,
    /// The folder listed; none for the folders scanned.
    folder: RefCell<Option<PathBuf>>,
    entries: RefCell<Vec<Entry>>,
    content: gtk::Stack,
    empty: adw::StatusPage,
    detail: gtk::ScrolledWindow,
    shown: RefCell<Option<PathBuf>>,
}

impl Browse {
    pub fn new(ctx: &Rc<Ctx>) -> Rc<Browse> {
        let up = gtk::Button::builder()
            .icon_name("go-up-symbolic")
            .tooltip_text("Up")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let title = ui::label("", &["heading"]);
        title.set_wrap(false);
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let subtitle = ui::label("", &["dim-label", "caption"]);
        subtitle.set_wrap(false);
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        let names = gtk::Box::new(gtk::Orientation::Vertical, 0);
        names.set_hexpand(true);
        names.append(&title);
        names.append(&subtitle);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.set_margin_top(6);
        bar.set_margin_bottom(6);
        bar.set_margin_start(6);
        bar.set_margin_end(12);
        bar.append(&up);
        bar.append(&names);

        let list = gtk::ListBox::new();
        list.add_css_class("navigation-sidebar");
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.append(&bar);
        sidebar.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        sidebar.append(
            &gtk::ScrolledWindow::builder()
                .child(&list)
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build(),
        );

        let empty = adw::StatusPage::builder()
            .icon_name("folder-documents-symbolic")
            .title("Choose a Document")
            .description("Its places, its text and its history are shown here.")
            .build();
        let detail = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let content = gtk::Stack::new();
        content.add_named(&empty, Some("empty"));
        content.add_named(&detail, Some("document"));

        let split = adw::NavigationSplitView::builder()
            .sidebar(&adw::NavigationPage::new(&sidebar, "Browse"))
            .content(&adw::NavigationPage::new(&content, "Document"))
            .min_sidebar_width(300.0)
            .max_sidebar_width(420.0)
            .sidebar_width_fraction(0.36)
            .build();

        let browse = Rc::new(Browse {
            ctx: ctx.clone(),
            widget: split.clone().upcast(),
            split,
            up,
            title,
            subtitle,
            list,
            folder: RefCell::new(None),
            entries: RefCell::new(Vec::new()),
            content,
            empty,
            detail,
            shown: RefCell::new(None),
        });
        {
            let weak = Rc::downgrade(&browse);
            browse.up.connect_clicked(move |_| {
                if let Some(browse) = weak.upgrade() {
                    browse.go_up();
                }
            });
        }
        {
            let weak = Rc::downgrade(&browse);
            browse.list.connect_row_selected(move |_, row| {
                if let (Some(browse), Some(row)) = (weak.upgrade(), row) {
                    browse.picked(row.index(), false);
                }
            });
        }
        {
            let weak = Rc::downgrade(&browse);
            browse.list.connect_row_activated(move |_, row| {
                if let Some(browse) = weak.upgrade() {
                    browse.picked(row.index(), true);
                }
            });
        }
        {
            let keys = gtk::EventControllerKey::new();
            let weak = Rc::downgrade(&browse);
            keys.connect_key_pressed(move |_, key, _, state| {
                let up = key == gtk::gdk::Key::BackSpace
                    || (key == gtk::gdk::Key::Up
                        && state.contains(gtk::gdk::ModifierType::ALT_MASK));
                match (up, weak.upgrade()) {
                    (true, Some(browse)) => {
                        browse.go_up();
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            });
            browse.list.add_controller(keys);
        }
        {
            let weak = Rc::downgrade(&browse);
            ctx.on_changed(move || {
                if let Some(browse) = weak.upgrade() {
                    browse.refresh();
                }
            });
        }
        browse.list_folder();
        browse
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.ctx.index.borrow().roots().unwrap_or_default()
    }

    fn go_up(&self) {
        let folder = self.folder.borrow().clone();
        let Some(folder) = folder else { return };
        let roots = self.roots();
        let up = match roots.contains(&folder) {
            true => None,
            false => folder.parent().map(Path::to_owned),
        };
        self.open_folder(up);
        // The folder come from is selected, so going down again is one key.
        let entries = self.entries.borrow();
        if let Some(i) = entries
            .iter()
            .position(|e| matches!(e, Entry::Folder(f) if *f == folder))
            && let Some(row) = self.list.row_at_index(i as i32)
        {
            row.grab_focus();
        }
    }

    fn open_folder(&self, folder: Option<PathBuf>) {
        *self.folder.borrow_mut() = folder;
        self.list_folder();
        self.list.grab_focus();
    }

    /// List again, keeping the folder and document shown while they are still there.
    fn refresh(&self) {
        let mut folder = self.folder.borrow().clone();
        while let Some(f) = &folder
            && !f.is_dir()
        {
            folder = f
                .parent()
                .map(Path::to_owned)
                .filter(|p| p.starts_with("/"));
        }
        *self.folder.borrow_mut() =
            folder.filter(|f| self.roots().iter().any(|r| f.starts_with(r)));
        self.list_folder();
        let shown = self.shown.borrow().clone();
        match shown {
            Some(path) if path.is_file() => self.show_file(&path),
            Some(_) => {
                self.shown.replace(None);
                self.content.set_visible_child_name("empty");
            }
            None => {}
        }
    }

    fn list_folder(&self) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let folder = self.folder.borrow().clone();
        self.up.set_sensitive(folder.is_some());
        let entries = match &folder {
            None => {
                self.title.set_text("Your Folders");
                self.subtitle.set_text("The folders clerk keeps track of");
                self.roots().into_iter().map(Entry::Folder).collect()
            }
            Some(folder) => {
                self.title.set_text(&ui::name_of(folder));
                self.subtitle.set_text(&ui::tilde(folder));
                match entries_of(folder) {
                    Ok(entries) => entries,
                    Err(e) => {
                        let note = ui::label(
                            &format!("This folder could not be read: {e}"),
                            &["dim-label"],
                        );
                        note.set_margin_top(12);
                        note.set_margin_start(12);
                        note.set_margin_end(12);
                        self.list.append(
                            &gtk::ListBoxRow::builder()
                                .child(&note)
                                .selectable(false)
                                .activatable(false)
                                .build(),
                        );
                        Vec::new()
                    }
                }
            }
        };
        let index = self.ctx.index.borrow();
        for entry in &entries {
            self.list.append(&match entry {
                Entry::Folder(path) => row(path, None, true),
                Entry::File(path) => {
                    let note = match index.hash_at(path) {
                        Ok(Some(_)) => None,
                        _ => Some("Not scanned yet"),
                    };
                    row(path, note, false)
                }
            });
        }
        if entries.is_empty() && folder.is_some() && self.list.row_at_index(0).is_none() {
            let note = ui::label("This folder is empty.", &["dim-label"]);
            note.set_margin_top(12);
            note.set_margin_start(12);
            self.list.append(
                &gtk::ListBoxRow::builder()
                    .child(&note)
                    .selectable(false)
                    .activatable(false)
                    .build(),
            );
        }
        drop(index);
        *self.entries.borrow_mut() = entries;
        // Keep the shown document selected when it is in this folder.
        let shown = self.shown.borrow().clone();
        if let Some(shown) = shown {
            let i = self
                .entries
                .borrow()
                .iter()
                .position(|e| matches!(e, Entry::File(f) if *f == shown));
            if let Some(row) = i.and_then(|i| self.list.row_at_index(i as i32)) {
                self.list.select_row(Some(&row));
            }
        }
    }

    /// A row was picked: a folder is opened when activated, a file shown when selected.
    fn picked(&self, i: i32, activated: bool) {
        let picked = match self.entries.borrow().get(i as usize) {
            Some(Entry::Folder(f)) => Some((f.clone(), true)),
            Some(Entry::File(f)) => Some((f.clone(), false)),
            None => None,
        };
        match picked {
            Some((folder, true)) if activated => self.open_folder(Some(folder)),
            Some((file, false)) => {
                if self.shown.borrow().as_deref() != Some(&file) {
                    self.show_file(&file);
                }
                if activated {
                    self.split.set_show_content(true);
                }
            }
            _ => {}
        }
    }

    fn show_file(&self, path: &Path) {
        self.shown.replace(Some(path.to_owned()));
        let hash = self.ctx.index.borrow().hash_at(path).ok().flatten();
        match hash {
            Some(hash) => {
                let page = document::page(
                    &self.ctx,
                    &hash,
                    path,
                    &[],
                    &self.split,
                    "Back to the Folder",
                );
                self.detail.set_child(Some(&page));
                self.content.set_visible_child_name("document");
            }
            None => {
                self.empty.set_title(&ui::name_of(path));
                self.empty.set_description(Some(
                    "clerk has not scanned this file yet. It is read on the next scan of its \
                     folder, which happens by itself while clerk is open.",
                ));
                self.content.set_visible_child_name("empty");
            }
        }
    }
}

/// What is in `folder`, folders first, each sorted as people sort names. Hidden ones are left
/// out, as a scan leaves them out.
fn entries_of(folder: &Path) -> std::io::Result<Vec<Entry>> {
    let mut folders = Vec::new();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(folder)?.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        match entry.file_type() {
            Ok(t) if t.is_dir() => folders.push(entry.path()),
            Ok(t) if t.is_file() => files.push(entry.path()),
            _ => {}
        }
    }
    let by_name = |a: &PathBuf, b: &PathBuf| {
        glib::FilenameCollationKey::from(ui::name_of(a).as_str())
            .cmp(&glib::FilenameCollationKey::from(ui::name_of(b).as_str()))
    };
    folders.sort_by(by_name);
    files.sort_by(by_name);
    Ok(folders
        .into_iter()
        .map(Entry::Folder)
        .chain(files.into_iter().map(Entry::File))
        .collect())
}

fn row(path: &Path, note: Option<&str>, folder: bool) -> gtk::ListBoxRow {
    let icon = match folder {
        true => gtk::Image::from_icon_name("folder-symbolic"),
        false => gtk::Image::from_gicon(&ui::file_icon(path)),
    };
    icon.set_pixel_size(if folder { 16 } else { 24 });
    icon.set_width_request(24);
    let name = ui::label(&ui::name_of(path), &[]);
    name.set_hexpand(true);
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 0);
    texts.set_valign(gtk::Align::Center);
    texts.append(&name);
    if let Some(note) = note {
        texts.append(&ui::label(note, &["dim-label", "caption"]));
    }
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    line.set_margin_top(4);
    line.set_margin_bottom(4);
    line.append(&icon);
    line.append(&texts);
    if folder {
        let go = gtk::Image::from_icon_name("go-next-symbolic");
        go.add_css_class("dim-label");
        line.append(&go);
    }
    gtk::ListBoxRow::builder().child(&line).build()
}
