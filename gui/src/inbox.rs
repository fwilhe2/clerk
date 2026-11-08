// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Inbox: where each file in Downloads belongs, and why, and moving it there when asked. Also
//! the journal of moves.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use clerk_core::{Advice, Error, InboxItem, Index};
use gtk::{gio, glib};

use crate::{Ctx, ui};

pub struct Inbox {
    ctx: Rc<Ctx>,
    pub widget: gtk::Widget,
    title: gtk::Label,
    summary: gtk::Label,
    file_all: gtk::Button,
    list: gtk::ListBox,
    stack: gtk::Stack,
    empty: adw::StatusPage,
    items: RefCell<Vec<InboxItem>>,
    page: RefCell<Option<adw::ViewStackPage>>,
    /// Which refresh is the latest, so an earlier one finishing late is dropped.
    generation: Cell<u64>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    pending: RefCell<Option<glib::SourceId>>,
}

impl Inbox {
    pub fn new(ctx: &Rc<Ctx>) -> Rc<Inbox> {
        let title = ui::label("", &["title-2"]);
        let summary = ui::label("", &["dim-label"]);
        let names = gtk::Box::new(gtk::Orientation::Vertical, 4);
        names.set_hexpand(true);
        names.append(&title);
        names.append(&summary);
        let choose = gtk::Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text("Look at Another Folder")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let file_all = gtk::Button::builder()
            .css_classes(["pill", "suggested-action"])
            .valign(gtk::Align::Center)
            .build();
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        head.append(&names);
        head.append(&choose);
        head.append(&file_all);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let note = ui::label(
            "Suggestions come from where you already keep files like these. Nothing moves \
             until you say so, a file is never put over another one, and every move can be \
             undone from the Journal of Moves.",
            &["dim-label", "caption"],
        );
        let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
        page.set_margin_top(24);
        page.set_margin_bottom(48);
        page.set_margin_start(16);
        page.set_margin_end(16);
        page.append(&head);
        page.append(&list);
        page.append(&note);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&adw::Clamp::builder().maximum_size(820).child(&page).build())
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("mail-inbox-symbolic")
            .build();
        let loading = adw::StatusPage::builder()
            .title("Looking at Your Downloads")
            .child(
                &gtk::Spinner::builder()
                    .spinning(true)
                    .width_request(32)
                    .height_request(32)
                    .build(),
            )
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(&loading, Some("loading"));
        stack.add_named(&scrolled, Some("list"));
        stack.add_named(&empty, Some("empty"));

        let inbox = Rc::new(Inbox {
            ctx: ctx.clone(),
            widget: stack.clone().upcast(),
            title,
            summary,
            file_all,
            list,
            stack,
            empty,
            items: RefCell::new(Vec::new()),
            page: RefCell::new(None),
            generation: Cell::new(0),
            monitor: RefCell::new(None),
            pending: RefCell::new(None),
        });
        {
            let weak = Rc::downgrade(&inbox);
            inbox.file_all.connect_clicked(move |_| {
                if let Some(inbox) = weak.upgrade() {
                    inbox.file_all();
                }
            });
        }
        {
            let weak = Rc::downgrade(&inbox);
            choose.connect_clicked(move |_| {
                let Some(inbox) = weak.upgrade() else { return };
                let dialog = gtk::FileDialog::builder()
                    .title("Look at Another Folder")
                    .initial_folder(&gio::File::for_path(&*inbox.ctx.inbox_folder.borrow()))
                    .build();
                let weak = Rc::downgrade(&inbox);
                dialog.select_folder(Some(&inbox.ctx.window), gio::Cancellable::NONE, move |r| {
                    if let (Some(inbox), Some(path)) =
                        (weak.upgrade(), r.ok().and_then(|f| f.path()))
                    {
                        *inbox.ctx.inbox_folder.borrow_mut() = path;
                        inbox.watch_folder();
                        inbox.refresh();
                    }
                });
            });
        }
        {
            let weak = Rc::downgrade(&inbox);
            ctx.on_changed(move || {
                if let Some(inbox) = weak.upgrade() {
                    inbox.refresh();
                }
            });
        }
        inbox.watch_folder();
        inbox.refresh();
        inbox
    }

    /// Show how many files have a suggestion on the Inbox's tab.
    pub fn set_badge(&self, page: adw::ViewStackPage) {
        *self.page.borrow_mut() = Some(page);
        self.update_badge();
    }

    fn update_badge(&self) {
        let n = self.suggested().len();
        if let Some(page) = &*self.page.borrow() {
            page.set_badge_number(n as u32);
            page.set_needs_attention(n > 0);
        }
    }

    /// Look again when something in the folder changes; a download arrives in pieces, so only
    /// once it has gone quiet.
    fn watch_folder(self: &Rc<Self>) {
        let folder = gio::File::for_path(&*self.ctx.inbox_folder.borrow());
        let monitor = folder
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            .ok();
        if let Some(monitor) = &monitor {
            let weak = Rc::downgrade(self);
            monitor.connect_changed(move |_, _, _, _| {
                let Some(inbox) = weak.upgrade() else { return };
                if let Some(id) = inbox.pending.borrow_mut().take() {
                    id.remove();
                }
                let again = Rc::downgrade(&inbox);
                let id = glib::timeout_add_local_once(Duration::from_secs(2), move || {
                    if let Some(inbox) = again.upgrade() {
                        inbox.pending.borrow_mut().take();
                        inbox.refresh();
                    }
                });
                *inbox.pending.borrow_mut() = Some(id);
            });
        }
        *self.monitor.borrow_mut() = monitor;
    }

    fn suggested(&self) -> Vec<(PathBuf, PathBuf)> {
        self.items
            .borrow()
            .iter()
            .filter_map(|item| match &item.advice {
                Advice::Suggest { folder, .. } => Some((item.path.clone(), folder.clone())),
                _ => None,
            })
            .collect()
    }

    /// Look at the folder again. Reading a large download takes a while, so it is read on
    /// another thread, with an index connection of its own.
    pub fn refresh(self: &Rc<Self>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        if self.items.borrow().is_empty() {
            self.stack.set_visible_child_name("loading");
        }
        let folder = self.ctx.inbox_folder.borrow().clone();
        let file = self.ctx.index_file.clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let f = folder.clone();
            let items = gio::spawn_blocking(move || {
                Index::open(&file)
                    .and_then(|index| index.inbox(&f))
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|_| Err("the folder could not be read".to_owned()));
            let Some(inbox) = weak.upgrade() else { return };
            if inbox.generation.get() == generation {
                inbox.show(&folder, items);
            }
        });
    }

    fn show(self: &Rc<Self>, folder: &Path, items: Result<Vec<InboxItem>, String>) {
        let name = ui::name_of(folder);
        let items = match items {
            Ok(items) => items,
            Err(e) => {
                self.items.borrow_mut().clear();
                self.update_badge();
                self.empty.set_title(&format!("{name} Could Not Be Read"));
                self.empty.set_description(Some(&e));
                self.stack.set_visible_child_name("empty");
                return;
            }
        };
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        for item in &items {
            self.list.append(&self.row(item));
        }
        let suggested = items
            .iter()
            .filter(|i| matches!(i.advice, Advice::Suggest { .. }))
            .count();
        let empty = items.is_empty();
        *self.items.borrow_mut() = items;
        self.update_badge();
        if empty {
            self.empty.set_title(&format!("Nothing in {name}"));
            self.empty.set_description(Some(&format!(
                "New files in {} are shown here, with where they belong.",
                ui::tilde(folder)
            )));
            self.stack.set_visible_child_name("empty");
            return;
        }
        let total = self.items.borrow().len();
        self.title.set_text(&name);
        self.summary.set_text(&format!(
            "{} {} in {} · {} with a suggestion",
            ui::count(total),
            if total == 1 { "file" } else { "files" },
            ui::tilde(folder),
            ui::count(suggested)
        ));
        self.file_all.set_visible(suggested > 1);
        self.file_all
            .set_label(&format!("File {} Suggested", ui::count(suggested)));
        self.stack.set_visible_child_name("list");
    }

    fn row(self: &Rc<Self>, item: &InboxItem) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(ui::name_of(&item.path))
            .use_markup(false)
            .subtitle_lines(0)
            .build();
        let icon = gtk::Image::from_gicon(&ui::file_icon(&item.path));
        icon.set_pixel_size(32);
        row.add_prefix(&icon);
        let button = |label: &str, suggested: bool| {
            let b = gtk::Button::builder()
                .label(label)
                .valign(gtk::Align::Center)
                .build();
            if suggested {
                b.add_css_class("suggested-action");
            }
            row.add_suffix(&b);
            b
        };
        match &item.advice {
            Advice::Suggest { folder, reason } => {
                row.set_subtitle(&format!("→ {}\n{reason}", ui::tilde(folder)));
                let file = button("File", true);
                let elsewhere = button("Elsewhere…", false);
                let (inbox, path, to) = (self.clone(), item.path.clone(), folder.clone());
                file.connect_clicked(move |_| inbox.move_files(vec![(path.clone(), to.clone())]));
                let (inbox, path) = (self.clone(), item.path.clone());
                elsewhere.connect_clicked(move |_| inbox.choose(&path));
            }
            Advice::Known(paths) => {
                row.set_subtitle(&format!(
                    "Already in your documents, at {}. This one is a copy.",
                    paths
                        .iter()
                        .map(|p| ui::tilde(p))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                row.add_css_class("dim-label");
                let show = button("Show", false);
                let (window, at) = (self.ctx.window.clone(), paths[0].clone());
                show.connect_clicked(move |_| ui::show_in_folder(&window, &at));
            }
            Advice::Nothing => {
                row.set_subtitle("No folder keeps files like this one yet.");
                let choose = button("Move To…", false);
                let (inbox, path) = (self.clone(), item.path.clone());
                choose.connect_clicked(move |_| inbox.choose(&path));
            }
        }
        row
    }

    fn choose(self: &Rc<Self>, path: &Path) {
        let start = self
            .ctx
            .index
            .borrow()
            .roots()
            .ok()
            .and_then(|r| r.into_iter().next());
        let dialog = gtk::FileDialog::builder()
            .title(format!("Move {}", ui::name_of(path)))
            .accept_label("Move Here")
            .build();
        if let Some(start) = start {
            dialog.set_initial_folder(Some(&gio::File::for_path(start)));
        }
        let (inbox, path) = (self.clone(), path.to_owned());
        dialog.select_folder(Some(&self.ctx.window), gio::Cancellable::NONE, move |r| {
            if let Some(folder) = r.ok().and_then(|f| f.path()) {
                inbox.move_files(vec![(path.clone(), folder)]);
            }
        });
    }

    fn file_all(self: &Rc<Self>) {
        self.move_files(self.suggested());
    }

    /// Move each file into its folder, asking first for those that would go to another drive.
    fn move_files(self: &Rc<Self>, moves: Vec<(PathBuf, PathBuf)>) {
        let mut done = Vec::new();
        let mut across = Vec::new();
        for (file, folder) in moves {
            match self.ctx.index.borrow_mut().move_file(&file, &folder, false) {
                Ok(m) => done.push(m.to),
                Err(Error::AcrossFileSystems { .. }) => across.push((file, folder)),
                Err(e) => self.ctx.toast(&format!("Not moved: {e}")),
            }
        }
        self.moved(&done);
        if across.is_empty() {
            return;
        }
        let (heading, body) = match across.as_slice() {
            [(file, folder)] => (
                "Move to Another Drive?".to_owned(),
                format!(
                    "{} is on another drive. clerk copies {} there, checks the copy, and then \
                     removes the original.",
                    ui::tilde(folder),
                    ui::name_of(file)
                ),
            ),
            many => (
                format!("Move {} Files to Another Drive?", many.len()),
                "Their folders are on another drive. clerk copies each file there, checks the \
                 copy, and then removes the original."
                    .to_owned(),
            ),
        };
        let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
        dialog.add_responses(&[("cancel", "Cancel"), ("move", "Copy and Move")]);
        dialog.set_response_appearance("move", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("move"));
        dialog.set_close_response("cancel");
        let inbox = self.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "move" {
                return;
            }
            let mut done = Vec::new();
            for (file, folder) in &across {
                match inbox.ctx.index.borrow_mut().move_file(file, folder, true) {
                    Ok(m) => done.push(m.to),
                    Err(e) => inbox.ctx.toast(&format!("Not moved: {e}")),
                }
            }
            inbox.moved(&done);
        });
        dialog.present(Some(&self.ctx.window));
    }

    /// Say what was moved, with a way to undo it.
    fn moved(&self, done: &[PathBuf]) {
        if done.is_empty() {
            return;
        }
        let title = match done {
            [one] => format!(
                "Moved {} to {}",
                ui::name_of(one),
                one.parent().map(ui::tilde).unwrap_or_default()
            ),
            many => format!("Moved {} files", many.len()),
        };
        let toast = adw::Toast::builder()
            .title(title)
            .use_markup(false)
            .button_label("Undo")
            .timeout(8)
            .build();
        let ctx = self.ctx.clone();
        let n = done.len();
        toast.connect_button_clicked(move |_| ctx.undo(n));
        self.ctx.toasts.add_toast(toast);
        self.ctx.changed();
    }
}

/// Every move clerk made, newest first, and a way to undo the last.
pub fn journal(ctx: &Rc<Ctx>) {
    let moves = ctx.index.borrow().journal().unwrap_or_default();
    let group = adw::PreferencesGroup::new();
    if moves.is_empty() {
        group.set_description(Some(
            "No moves yet. A file you file from the Inbox is listed here, and can be moved back.",
        ));
    }
    for m in moves.iter().rev() {
        let mut subtitle = format!(
            "{} · from {}",
            ui::local_time(m.at),
            m.from.parent().map(ui::tilde).unwrap_or_default()
        );
        if m.copied {
            subtitle += " · copied to another drive";
        }
        let row = adw::ActionRow::builder()
            .title(ui::tilde(&m.to))
            .subtitle(subtitle)
            .use_markup(false)
            .build();
        if let Some(at) = m.undone {
            let tag = gtk::Label::new(Some("Undone"));
            tag.add_css_class("dim-label");
            tag.add_css_class("caption-heading");
            tag.set_tooltip_text(Some(&format!("Moved back {}", ui::local_time(at))));
            row.add_suffix(&tag);
            row.add_css_class("dim-label");
        }
        group.add(&row);
    }
    let page = adw::PreferencesPage::new();
    page.add(&group);
    let undo = gtk::Button::builder()
        .label("Undo Last Move")
        .sensitive(moves.iter().any(|m| m.undone.is_none()))
        .build();
    let header = adw::HeaderBar::new();
    header.pack_start(&undo);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title("Journal of Moves")
        .content_width(560)
        .content_height(560)
        .child(&view)
        .build();
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        undo.connect_clicked(move |_| {
            dialog.close();
            ctx.undo(1);
        });
    }
    dialog.present(Some(&ctx.window));
}
