// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Folders: what is indexed and when it was scanned; and the first run, which is choosing the
//! first folder.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use clerk_core::Progress;
use gtk::{gio, glib};

use crate::{Ctx, ui};

pub struct Folders {
    ctx: Rc<Ctx>,
    pub widget: gtk::Widget,
    page: adw::PreferencesPage,
    groups: RefCell<Vec<adw::PreferencesGroup>>,
}

impl Folders {
    pub fn new(ctx: &Rc<Ctx>) -> Rc<Folders> {
        let page = adw::PreferencesPage::new();
        let folders = Rc::new(Folders {
            ctx: ctx.clone(),
            widget: page.clone().upcast(),
            page,
            groups: RefCell::new(Vec::new()),
        });
        {
            let weak = Rc::downgrade(&folders);
            ctx.on_changed(move || {
                if let Some(folders) = weak.upgrade() {
                    folders.refresh();
                }
            });
        }
        folders.refresh();
        folders
    }

    fn refresh(&self) {
        for group in self.groups.borrow_mut().drain(..) {
            self.page.remove(&group);
        }
        let status = match self.ctx.index.borrow().status() {
            Ok(status) => status,
            Err(e) => {
                let group = adw::PreferencesGroup::builder()
                    .title("The Index Could Not Be Read")
                    .description(e.to_string())
                    .build();
                self.add(group);
                return;
            }
        };

        let docs = adw::PreferencesGroup::builder()
            .title("Your Documents")
            .build();
        let number = |title: &str, n: usize, subtitle: Option<&str>| {
            let row = adw::ActionRow::builder().title(title).build();
            if let Some(subtitle) = subtitle {
                row.set_subtitle(subtitle);
            }
            let value = gtk::Label::new(Some(&ui::count(n)));
            value.add_css_class("numeric");
            value.add_css_class("heading");
            row.add_suffix(&value);
            docs.add(&row);
        };
        number(
            "Documents",
            status.documents,
            Some(&format!("in {} files", ui::count(status.files))),
        );
        number("With Their Text Read", status.with_text, None);
        if status.awaiting_ocr > 0 {
            number(
                "Waiting to Be Read",
                status.awaiting_ocr,
                Some(
                    "Scans and pictures. Install tesseract, and poppler's pdftoppm for PDFs, \
                     and clerk reads them on the next scan, on this computer.",
                ),
            );
        }
        self.add(docs);

        let roots = adw::PreferencesGroup::builder()
            .title("Folders")
            .description(
                "clerk keeps these current while it is open. It only reads them, and never \
                 changes a file unless you ask.",
            )
            .build();
        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Add Folder")
            .css_classes(["flat"])
            .build();
        {
            let ctx = self.ctx.clone();
            add.connect_clicked(move |_| choose_folder(&ctx, "Add Folder"));
        }
        roots.set_header_suffix(Some(&add));
        for root in &status.roots {
            let scanned = root.scanned.map_or("never scanned".to_owned(), |at| {
                format!("scanned {}", ui::local_time(at))
            });
            let row = adw::ActionRow::builder()
                .title(ui::tilde(&root.path))
                .subtitle(format!("{} files · {scanned}", ui::count(root.files)))
                .use_markup(false)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
            let again = gtk::Button::builder()
                .icon_name("view-refresh-symbolic")
                .tooltip_text("Scan Again")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let (ctx, path) = (self.ctx.clone(), root.path.clone());
            again.connect_clicked(move |_| ctx.worker.scan(path.clone()));
            row.add_suffix(&again);
            roots.add(&row);
        }
        self.add(roots);

        let index = adw::PreferencesGroup::builder()
            .title("Index")
            .description(
                "What clerk knows is kept here, outside your folders. Deleting it loses \
                 nothing that is in them.",
            )
            .build();
        let row = adw::ActionRow::builder()
            .title(ui::tilde(&status.index))
            .use_markup(false)
            .build();
        let show = gtk::Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text("Show in Folder")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let (window, file) = (self.ctx.window.clone(), status.index.clone());
        show.connect_clicked(move |_| ui::show_in_folder(&window, &file));
        row.add_suffix(&show);
        index.add(&row);
        self.add(index);
    }

    fn add(&self, group: adw::PreferencesGroup) {
        self.page.add(&group);
        self.groups.borrow_mut().push(group);
    }
}

/// Ask for a folder, and scan it.
fn choose_folder(ctx: &Rc<Ctx>, title: &str) {
    let dialog = gtk::FileDialog::builder()
        .title(title)
        .accept_label("Scan")
        .initial_folder(&gio::File::for_path(glib::home_dir()))
        .build();
    let ctx2 = ctx.clone();
    dialog.select_folder(Some(&ctx.window), gio::Cancellable::NONE, move |r| {
        if let Some(path) = r.ok().and_then(|f| f.path()) {
            ctx2.worker.scan(path);
        }
    });
}

/// The first run: a folder to choose, and how far its first scan is.
#[derive(Clone)]
pub struct Welcome {
    pub widget: gtk::Widget,
    status: adw::StatusPage,
    buttons: gtk::Box,
    progress: gtk::Label,
}

impl Welcome {
    pub fn new(ctx: &Rc<Ctx>) -> Welcome {
        let documents = glib::user_special_dir(glib::UserDirectory::Documents)
            .unwrap_or_else(|| glib::home_dir().join("Documents"));
        let buttons = gtk::Box::new(gtk::Orientation::Vertical, 12);
        buttons.set_halign(gtk::Align::Center);
        if documents.is_dir() {
            let use_docs = gtk::Button::builder()
                .label(format!("Use {}", ui::tilde(&documents)))
                .css_classes(["pill", "suggested-action"])
                .build();
            let ctx = ctx.clone();
            let folder: PathBuf = documents.clone();
            use_docs.connect_clicked(move |_| ctx.worker.scan(folder.clone()));
            buttons.append(&use_docs);
        }
        let other = gtk::Button::builder()
            .label("Choose Another Folder…")
            .css_classes(["pill"])
            .build();
        {
            let ctx = ctx.clone();
            other.connect_clicked(move |_| choose_folder(&ctx, "Choose Your Documents Folder"));
        }
        buttons.append(&other);
        let progress = ui::label("", &["dim-label", "numeric"]);
        progress.set_xalign(0.5);
        progress.set_visible(false);
        let child = gtk::Box::new(gtk::Orientation::Vertical, 12);
        child.append(&buttons);
        child.append(&progress);
        let status = adw::StatusPage::builder()
            .icon_name("folder-documents-symbolic")
            .title("Keep Track of Your Documents")
            .description(
                "clerk reads the files in a folder, so you can find them by name and by their \
                 text, and follows them wherever they are moved. It never changes, moves or \
                 deletes a file unless you ask, and nothing leaves this computer.",
            )
            .child(&child)
            .vexpand(true)
            .build();
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&status));
        Welcome {
            widget: view.upcast(),
            status,
            buttons,
            progress,
        }
    }

    pub fn progress(&self, progress: Progress) {
        self.buttons.set_visible(false);
        self.progress.set_visible(true);
        self.status.set_title("Reading Your Documents");
        self.status.set_description(Some(
            "The first scan reads every file and its text, which can take a few minutes. \
             Later scans read only what changed.",
        ));
        self.progress.set_text(&match progress {
            Progress::Files(n) => format!("{} files found", ui::count(n)),
            Progress::Texts(n, of) => format!(
                "Text read of {} of {} documents",
                ui::count(n),
                ui::count(of)
            ),
        });
    }

    pub fn failed(&self, why: &str) {
        self.buttons.set_visible(true);
        self.progress.set_visible(false);
        self.status.set_title("That Folder Could Not Be Scanned");
        self.status.set_description(Some(why));
    }
}
