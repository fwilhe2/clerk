// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The thread that scans and watches, with an index connection of its own, so the window stays
//! responsive while a first scan reads every file.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use clerk_core::{Index, Progress, Report};

/// What the worker tells the window.
pub enum Msg {
    Progress(PathBuf, Progress),
    Scanned(Result<Report, String>),
    /// Every folder is scanned, and watched until something changes.
    Watching,
    /// Not watching, and why; scanning on request still works.
    Stopped(String),
}

pub struct Worker {
    tx: mpsc::Sender<PathBuf>,
    pending: Arc<AtomicBool>,
}

impl Worker {
    /// Start the worker on the index at `file`. It scans every folder scanned before, then
    /// watches them.
    pub fn start(file: PathBuf) -> (Worker, async_channel::Receiver<Msg>) {
        let (tx, rx) = mpsc::channel();
        let (out, msgs) = async_channel::unbounded();
        let pending = Arc::new(AtomicBool::new(false));
        let flag = pending.clone();
        std::thread::spawn(move || run(file, rx, flag, out));
        (Worker { tx, pending }, msgs)
    }

    /// Scan `folder` now, and watch it from then on.
    pub fn scan(&self, folder: PathBuf) {
        self.pending.store(true, Ordering::SeqCst);
        let _ = self.tx.send(folder);
    }
}

fn run(
    file: PathBuf,
    rx: mpsc::Receiver<PathBuf>,
    pending: Arc<AtomicBool>,
    out: async_channel::Sender<Msg>,
) {
    let send = |msg| {
        let _ = out.send_blocking(msg);
    };
    let mut index = match Index::open(&file) {
        Ok(index) => index,
        Err(e) => return send(Msg::Stopped(e.to_string())),
    };
    let mut queue = index.roots().unwrap_or_default();
    loop {
        pending.store(false, Ordering::SeqCst);
        queue.extend(rx.try_iter());
        queue.dedup();
        for root in queue.drain(..) {
            let report = index.scan(&root, |p| send(Msg::Progress(root.clone(), p)));
            send(Msg::Scanned(report.map_err(|e| e.to_string())));
        }
        if pending.load(Ordering::SeqCst) {
            continue;
        }
        if index.roots().map(|r| r.is_empty()).unwrap_or(true) {
            match rx.recv() {
                Ok(folder) => queue.push(folder),
                Err(_) => return,
            }
            continue;
        }
        send(Msg::Watching);
        let watched = index.watch(
            || pending.load(Ordering::SeqCst),
            |report| send(Msg::Scanned(report.map_err(|e| e.to_string()))),
        );
        if let Err(e) = watched {
            send(Msg::Stopped(e.to_string()));
            match rx.recv() {
                Ok(folder) => queue.push(folder),
                Err(_) => return,
            }
        }
    }
}
