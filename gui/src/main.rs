// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `clerk-gtk`: the clerk app for GNOME. It calls clerk's core and owns nothing; whatever it
//! does, the `clerk` command does too.

mod browse;
mod document;
mod find;
mod folders;
mod inbox;
mod ui;
mod worker;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use clerk_core::{Index, Progress};
use gtk::{gdk, gio, glib};

use crate::worker::{Msg, Worker};

const APP_ID: &str = "io.github.fwilhe2.Clerk";

const USAGE: &str = "Usage: clerk-gtk [--index FILE] [--inbox FOLDER]

  --index FILE     The index file, instead of ~/.local/share/clerk/index.sqlite
  --inbox FOLDER   The folder the Inbox looks at, instead of Downloads";

/// What every part of the window shares.
pub struct Ctx {
    pub index: RefCell<Index>,
    pub index_file: PathBuf,
    pub inbox_folder: RefCell<PathBuf>,
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    pub worker: Worker,
    /// Told whenever the index changed, by a scan or a move.
    changed: RefCell<Vec<Box<dyn Fn()>>>,
}

impl Ctx {
    pub fn on_changed(&self, f: impl Fn() + 'static) {
        self.changed.borrow_mut().push(Box::new(f));
    }

    pub fn changed(&self) {
        for f in self.changed.borrow().iter() {
            f();
        }
    }

    pub fn toast(&self, title: &str) {
        let toast = adw::Toast::new(title);
        toast.set_use_markup(false);
        self.toasts.add_toast(toast);
    }

    /// Undo the last `n` moves, saying what happened.
    pub fn undo(&self, n: usize) {
        let mut done = 0;
        for _ in 0..n {
            match self.index.borrow_mut().undo() {
                Ok(_) => done += 1,
                Err(e) => {
                    self.toast(&format!("Not moved back: {e}"));
                    break;
                }
            }
        }
        match done {
            0 => {}
            1 => self.toast("Moved back"),
            n => self.toast(&format!("Moved {n} files back")),
        }
        self.changed();
    }
}

struct Args {
    index: Option<PathBuf>,
    inbox: Option<PathBuf>,
}

fn args() -> Result<Args, String> {
    let mut args = Args {
        index: std::env::var_os("CLERK_INDEX").map(PathBuf::from),
        inbox: None,
    };
    let mut it = std::env::args_os().skip(1);
    while let Some(arg) = it.next() {
        match arg.to_str() {
            Some("--index") => args.index = Some(it.next().ok_or(USAGE)?.into()),
            Some("--inbox") => args.inbox = Some(it.next().ok_or(USAGE)?.into()),
            Some("-h" | "--help") => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ => return Err(USAGE.to_owned()),
        }
    }
    Ok(args)
}

fn main() -> glib::ExitCode {
    let args = match args() {
        Ok(args) => args,
        Err(usage) => {
            eprintln!("{usage}");
            return glib::ExitCode::FAILURE;
        }
    };
    clerk_core::quiet_reader_panics();
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let args = Rc::new(args);
    app.connect_activate(move |app| {
        if let Err(e) = build(app, &args) {
            eprintln!("clerk-gtk: {e}");
            app.quit();
        }
    });
    // The arguments are read above; GTK is given none of them.
    app.run_with_args::<&str>(&[])
}

fn build(app: &adw::Application, args: &Args) -> Result<(), String> {
    let index_file = args
        .index
        .clone()
        .or_else(Index::default_path)
        .ok_or("no home directory to keep the index in; name one with --index")?;
    let index = Index::open(&index_file).map_err(|e| e.to_string())?;
    let first_run = index.roots().map_err(|e| e.to_string())?.is_empty();
    let inbox_folder = args.inbox.clone().unwrap_or_else(|| {
        glib::user_special_dir(glib::UserDirectory::Downloads)
            .unwrap_or_else(|| glib::home_dir().join("Downloads"))
    });

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("clerk")
        .default_width(1080)
        .default_height(720)
        .width_request(360)
        .height_request(400)
        .build();
    let toasts = adw::ToastOverlay::new();
    let (worker, msgs) = Worker::start(index_file.clone());
    let ctx = Rc::new(Ctx {
        index: RefCell::new(index),
        index_file,
        inbox_folder: RefCell::new(inbox_folder),
        window: window.clone(),
        toasts: toasts.clone(),
        worker,
        changed: RefCell::new(Vec::new()),
    });
    ui::load_css();

    // The main view: Find, Inbox and Folders.
    let find = find::Find::new(&ctx);
    let browse = browse::Browse::new(&ctx);
    let inbox = inbox::Inbox::new(&ctx);
    let folders = folders::Folders::new(&ctx);
    let stack = adw::ViewStack::new();
    stack.add_titled_with_icon(&find.widget, Some("find"), "Find", "system-search-symbolic");
    stack.add_titled_with_icon(
        &browse.widget,
        Some("browse"),
        "Browse",
        "folder-open-symbolic",
    );
    let inbox_page =
        stack.add_titled_with_icon(&inbox.widget, Some("inbox"), "Inbox", "mail-inbox-symbolic");
    stack.add_titled_with_icon(
        &folders.widget,
        Some("folders"),
        "Folders",
        "folder-symbolic",
    );
    inbox.set_badge(inbox_page);

    let switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    let header = adw::HeaderBar::builder().title_widget(&switcher).build();
    let status = Status::new();
    header.pack_start(&status.button);
    let menu = gio::Menu::new();
    menu.append(Some("Journal of Moves"), Some("win.journal"));
    menu.append(Some("About clerk"), Some("win.about"));
    header.pack_end(
        &gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .primary(true)
            .tooltip_text("Main Menu")
            .build(),
    );
    let bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
    let main = adw::ToolbarView::new();
    main.add_top_bar(&header);
    main.add_bottom_bar(&bar);
    main.set_content(Some(&stack));
    {
        let stack = stack.clone();
        status
            .button
            .connect_clicked(move |_| stack.set_visible_child_name("folders"));
    }

    // The first run: one folder to choose, then its first scan.
    let welcome = folders::Welcome::new(&ctx);
    let outer = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    outer.add_named(&welcome.widget, Some("welcome"));
    outer.add_named(&main, Some("main"));
    outer.set_visible_child_name(if first_run { "welcome" } else { "main" });
    toasts.set_child(Some(&outer));
    window.set_content(Some(&toasts));

    let narrow = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 640sp").expect("a valid condition"),
    );
    narrow.add_setter(&switcher, "visible", Some(&false.to_value()));
    narrow.add_setter(&bar, "reveal", Some(&true.to_value()));
    narrow.add_setter(&find.split, "collapsed", Some(&true.to_value()));
    narrow.add_setter(&browse.split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(narrow);

    actions(&ctx, &stack, &find);
    drop_target(&ctx, &stack, &find);

    // What the worker says.
    {
        let ctx = ctx.clone();
        let welcome = welcome.clone();
        let outer = outer.clone();
        let scanning = Cell::new(false);
        glib::spawn_future_local(async move {
            while let Ok(msg) = msgs.recv().await {
                match msg {
                    Msg::Progress(root, progress) => {
                        scanning.set(true);
                        status.scanning(&root, progress);
                        welcome.progress(progress);
                    }
                    Msg::Scanned(Ok(report)) => {
                        if scanning.replace(false) && report.added.len() + report.removed.len() > 0
                        {
                            ctx.toast(&ui::report_line(&report));
                        }
                        if outer.visible_child_name().as_deref() == Some("welcome") {
                            outer.set_visible_child_name("main");
                        }
                        ctx.changed();
                    }
                    Msg::Scanned(Err(e)) => {
                        scanning.set(false);
                        welcome.failed(&e);
                        ctx.toast(&format!("Not scanned: {e}"));
                    }
                    Msg::Watching => status.watching(),
                    Msg::Stopped(why) => status.stopped(&why),
                }
            }
        });
    }

    window.present();
    find.focus();
    Ok(())
}

fn actions(ctx: &Rc<Ctx>, stack: &adw::ViewStack, find: &Rc<find::Find>) {
    let journal = gio::SimpleAction::new("journal", None);
    {
        let ctx = ctx.clone();
        journal.connect_activate(move |_, _| inbox::journal(&ctx));
    }
    let about = gio::SimpleAction::new("about", None);
    {
        let window = ctx.window.clone();
        about.connect_activate(move |_, _| {
            adw::AboutDialog::builder()
                .application_name("clerk")
                .application_icon("folder-documents-symbolic")
                .version(env!("CARGO_PKG_VERSION"))
                .comments(
                    "Keeps track of your documents, wherever they are in your documents folder. \
                     clerk reads your files and never changes, moves or deletes one unless you \
                     ask. Its index is kept outside the folder, and nothing is sent anywhere.",
                )
                .license_type(gtk::License::Agpl30)
                .website("https://github.com/fwilhe2/clerk")
                .build()
                .present(Some(&window));
        });
    }
    let search = gio::SimpleAction::new("find", None);
    {
        let stack = stack.clone();
        let find = find.clone();
        search.connect_activate(move |_, _| {
            stack.set_visible_child_name("find");
            find.focus();
        });
    }
    ctx.window.add_action(&journal);
    ctx.window.add_action(&about);
    ctx.window.add_action(&search);
    if let Some(app) = ctx.window.application() {
        app.set_accels_for_action("win.find", &["<Control>f"]);
        app.set_accels_for_action("window.close", &["<Control>w"]);
    }
}

/// A file dropped on the window is looked up by its content, wherever it came from.
fn drop_target(ctx: &Rc<Ctx>, stack: &adw::ViewStack, find: &Rc<find::Find>) {
    let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    ctx.window.add_controller(target.clone());
    let ctx = ctx.clone();
    let stack = stack.clone();
    let find = find.clone();
    target.connect_drop(move |_, value, _, _| {
        let Ok(files) = value.get::<gdk::FileList>() else {
            return false;
        };
        let Some(path) = files.files().first().and_then(|f| f.path()) else {
            return false;
        };
        let ctx = ctx.clone();
        let stack = stack.clone();
        let find = find.clone();
        glib::spawn_future_local(async move {
            let p = path.clone();
            let hash = gio::spawn_blocking(move || clerk_core::hash_file(&p)).await;
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            match hash {
                Ok(Ok(hash)) => {
                    let known = ctx.index.borrow().paths_of(&hash).unwrap_or_default();
                    if known.is_empty() {
                        ctx.toast(&format!("{name} is not in your documents"));
                    } else {
                        stack.set_visible_child_name("find");
                        find.show_document(&hash, &known[0], &[]);
                    }
                }
                Ok(Err(e)) => ctx.toast(&format!("{name} could not be read: {e}")),
                Err(_) => {}
            }
        });
        true
    });
}

/// The header's word on whether the index is current.
struct Status {
    button: gtk::Button,
    spinner: gtk::Spinner,
    dot: gtk::Image,
    label: gtk::Label,
}

impl Status {
    fn new() -> Status {
        let spinner = gtk::Spinner::new();
        let dot = gtk::Image::from_icon_name("emblem-ok-symbolic");
        dot.add_css_class("success");
        let label = gtk::Label::new(Some("Starting"));
        label.add_css_class("caption");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&spinner);
        row.append(&dot);
        row.append(&label);
        let button = gtk::Button::builder()
            .child(&row)
            .tooltip_text("Your folders, and how current the index is")
            .build();
        button.add_css_class("flat");
        dot.set_visible(false);
        spinner.set_spinning(true);
        Status {
            button,
            spinner,
            dot,
            label,
        }
    }

    fn scanning(&self, root: &std::path::Path, progress: Progress) {
        self.spinner.set_visible(true);
        self.spinner.set_spinning(true);
        self.dot.set_visible(false);
        let name = root.file_name().unwrap_or_default().to_string_lossy();
        self.label.set_text(&match progress {
            Progress::Files(n) => format!("Scanning {name} · {} files", ui::count(n)),
            Progress::Texts(n, of) => {
                format!("Reading {name} · {} of {}", ui::count(n), ui::count(of))
            }
        });
    }

    fn watching(&self) {
        self.spinner.set_spinning(false);
        self.spinner.set_visible(false);
        self.dot.set_visible(true);
        self.dot.set_icon_name(Some("emblem-ok-symbolic"));
        self.dot.remove_css_class("warning");
        self.dot.add_css_class("success");
        self.label.set_text("Up to date");
        self.button.set_tooltip_text(Some(
            "clerk follows changes in your folders while it is open",
        ));
    }

    fn stopped(&self, why: &str) {
        self.spinner.set_spinning(false);
        self.spinner.set_visible(false);
        self.dot.set_visible(true);
        self.dot.set_icon_name(Some("dialog-warning-symbolic"));
        self.dot.remove_css_class("success");
        self.dot.add_css_class("warning");
        self.label.set_text("Not watching");
        self.button.set_tooltip_text(Some(why));
    }
}
