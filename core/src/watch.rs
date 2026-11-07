// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Keeping the index current while clerk is open: every scanned folder is watched, and one is
//! scanned again a moment after something in it changes. Only while it runs; clerk is not a
//! daemon (`doc/plan.md`, "Not doing"). Watching reads, as a scan does (C1).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecursiveMode, Watcher};

use crate::{Error, Index, Report, Result};

/// How long a folder must be quiet before it is scanned: a copy of many files is one scan.
const QUIET: Duration = Duration::from_secs(1);

/// How long a folder that never goes quiet waits for its scan at most.
const MAX_WAIT: Duration = Duration::from_secs(30);

/// How often `stop` is asked.
const TICK: Duration = Duration::from_millis(200);

impl Index {
    /// Scan every folder scanned before, then watch them, and scan one again each time
    /// something in it changed and it has gone quiet — until `stop` says to. `scanned` is told
    /// each scan's outcome.
    ///
    /// ponytail: a change scans the whole folder it is in, which reads only what changed but
    /// walks every directory; following the changed paths alone would lift that.
    pub fn watch(
        &mut self,
        stop: impl Fn() -> bool,
        mut scanned: impl FnMut(Result<Report>),
    ) -> Result<()> {
        let roots = self.roots()?;
        if roots.is_empty() {
            return Err(Error::NothingScanned);
        }
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
        })
        .map_err(|e| Error::Watch(e.to_string()))?;
        for root in &roots {
            watcher
                .watch(root, RecursiveMode::Recursive)
                .map_err(|e| Error::Watch(format!("{}: {e}", root.display())))?;
        }
        for root in &roots {
            scanned(self.scan(root, |_| {}));
        }

        // A changed folder, when it first changed, and when last.
        let mut dirty: HashMap<PathBuf, (Instant, Instant)> = HashMap::new();
        loop {
            match rx.recv_timeout(TICK) {
                Ok(Ok(event)) if !changes(&event.kind) => {}
                Ok(Ok(event)) => {
                    let now = Instant::now();
                    for path in &event.paths {
                        if let Some(root) = root_of(&roots, path) {
                            dirty.entry(root.clone()).or_insert((now, now)).1 = now;
                        }
                    }
                }
                // Events were lost, the kernel's queue overflowing: every folder is scanned.
                Ok(Err(_)) => {
                    let now = Instant::now();
                    for root in &roots {
                        dirty.entry(root.clone()).or_insert((now, now)).1 = now;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(Error::Watch("the watch ended".to_owned()));
                }
            }
            if stop() {
                return Ok(());
            }
            let now = Instant::now();
            let due: Vec<PathBuf> = dirty
                .iter()
                .filter(|(_, (first, last))| now - *last >= QUIET || now - *first >= MAX_WAIT)
                .map(|(root, _)| root.clone())
                .collect();
            for root in due {
                dirty.remove(&root);
                scanned(self.scan(&root, |_| {}));
            }
        }
    }
}

/// Whether an event is a change. Opening and reading are not: a scan opens every folder and
/// every file it reads, and counting that would have each scan cause the next.
fn changes(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

/// The folder `path` is in, of `roots`, unless it is hidden below it: a scan leaves hidden
/// entries out, so their changes, an editor's lock files among them, are no reason to scan.
fn root_of<'a>(roots: &'a [PathBuf], path: &Path) -> Option<&'a PathBuf> {
    let root = roots.iter().find(|root| path.starts_with(root))?;
    let hidden =
        path.strip_prefix(root).ok()?.components().any(
            |c| matches!(c, Component::Normal(name) if name.to_string_lossy().starts_with('.')),
        );
    (!hidden).then_some(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_is_not_a_change() {
        use notify::event::{CreateKind, ModifyKind, RenameMode};
        assert!(!changes(&EventKind::Access(AccessKind::Open(
            AccessMode::Any
        ))));
        assert!(!changes(&EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(changes(&EventKind::Access(AccessKind::Close(
            AccessMode::Write
        ))));
        assert!(changes(&EventKind::Create(CreateKind::File)));
        assert!(changes(&EventKind::Modify(ModifyKind::Name(
            RenameMode::Both
        ))));
    }

    #[test]
    fn a_hidden_change_is_no_reason_to_scan() {
        let roots = [PathBuf::from("/d/Documents")];
        assert_eq!(
            root_of(&roots, Path::new("/d/Documents/a/letter.odt")),
            Some(&roots[0])
        );
        assert_eq!(
            root_of(&roots, Path::new("/d/Documents/a/.~lock.letter.odt#")),
            None
        );
        assert_eq!(root_of(&roots, Path::new("/d/Documents/.git/index")), None);
        assert_eq!(root_of(&roots, Path::new("/d/Downloads/x.pdf")), None);
    }
}
