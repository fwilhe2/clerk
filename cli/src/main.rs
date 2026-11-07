// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `clerk`: keeps track of the documents in a directory, without ever changing them.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use clerk_core::{Advice, DocumentText, Index, Progress, Report, format_utc, hash_file};

/// Keeps track of your documents, wherever they are in your documents folder. clerk reads your
/// files and never changes, moves or deletes one unless you ask. Its index is kept outside the
/// folder, and nothing is sent anywhere.
#[derive(Parser)]
#[command(name = "clerk", version)]
struct Cli {
    /// The index file, instead of ~/.local/share/clerk/index.sqlite
    #[arg(long, global = true, env = "CLERK_INDEX", value_name = "FILE")]
    index: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Index a folder, and remember it
    ///
    /// The first scan reads every file, and the text of every document, which can take a few
    /// minutes; later scans read only what changed. With no folder, every folder scanned before is scanned again, or
    /// ~/Documents the first time. Hidden files and folders are left out.
    Scan {
        folders: Vec<PathBuf>,
        /// List every file added, moved, copied, changed or removed
        #[arg(long, short)]
        verbose: bool,
    },
    /// Find documents where every word is in the path or in the text
    ///
    /// Case is ignored, and a word is found inside longer words too: "auszug" finds
    /// "Kontoauszug". In the text, a word needs at least three letters.
    Find {
        #[arg(required = true)]
        words: Vec<String>,
        /// Show, under each document found by its text, where in the text
        #[arg(long, short)]
        context: bool,
    },
    /// The text clerk read from a document, which is what find searches
    ///
    /// The file is recognised by its content, so a copy of it anywhere works too.
    Text { file: PathBuf },
    /// Where a document is now, and where it has been
    ///
    /// The file is recognised by its content, so a copy of it anywhere works too.
    Where { file: PathBuf },
    /// Where each file in a folder like Downloads belongs, and why
    ///
    /// Learned from where you already keep files like it: files named the same way, or
    /// documents sharing a word, such as a customer number, that no other folder's documents
    /// have. A file already in your documents is said to be. Nothing is moved.
    Inbox {
        /// The folder to look at, ~/Downloads if none is named
        folder: Option<PathBuf>,
    },
    /// Move files into a folder, keeping their names
    ///
    /// A file already there by that name is never replaced, and every move can be undone with
    /// clerk undo.
    Move {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// The folder to move them into
        #[arg(long, short)]
        to: PathBuf,
        /// Allow moving to another drive or file system, by copying, checking the copy, and
        /// then removing the original
        #[arg(long)]
        across: bool,
    },
    /// Move files to where clerk inbox suggests
    ///
    /// A file with no suggestion, or already in your documents, is left where it is. Every
    /// move can be undone with clerk undo.
    File {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Allow moving to another drive or file system, by copying, checking the copy, and
        /// then removing the original
        #[arg(long)]
        across: bool,
    },
    /// Undo the last move clerk made
    ///
    /// Run it again to undo the one before. A file that changed since it was moved, or whose
    /// old place is taken, is left alone.
    Undo,
    /// Every move clerk made, and which were undone
    Journal,
    /// Keep the index current while this runs
    ///
    /// Scans every folder scanned before, then scans one again a moment after something in it
    /// changes, until stopped with Ctrl+C. Hidden files are left out, as in a scan.
    Watch {
        /// List every file added, moved, copied, changed or removed
        #[arg(long, short)]
        verbose: bool,
    },
    /// What is indexed, and when it was last scanned
    Status,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    clerk_core::quiet_reader_panics();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("clerk: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let file = cli
        .index
        .or_else(Index::default_path)
        .ok_or("no home directory to keep the index in; name one with --index")?;
    let mut index = Index::open(&file).map_err(|e| e.to_string())?;
    match cli.command {
        Command::Scan { folders, verbose } => scan(&mut index, folders, verbose),
        Command::Find { words, context } => {
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            for found in index.find(&words).map_err(|e| e.to_string())? {
                println!("{}", found.path.display());
                if let Some(text) = found.context.filter(|_| context) {
                    println!("\t{text}");
                }
            }
            Ok(())
        }
        Command::Text { file } => {
            let hash = hash_file(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            match index.text(&hash).map_err(|e| e.to_string())? {
                DocumentText::Read { text, .. } => {
                    println!("{}", text.trim_end());
                    Ok(())
                }
                DocumentText::Unread => Err(format!(
                    "{} has not been read; scan the folder it is in",
                    file.display()
                )),
                DocumentText::AwaitingOcr => Err(format!(
                    "{} is a scan or a picture; install tesseract (and poppler's pdftoppm for a \
                     PDF) and scan again to read it",
                    file.display()
                )),
                DocumentText::None => Err(format!(
                    "{}: clerk does not read text from this kind of file",
                    file.display()
                )),
                DocumentText::Failed { reader, why } => Err(format!(
                    "{}: its text could not be read ({reader}): {why}",
                    file.display()
                )),
            }
        }
        Command::Where { file } => {
            let hash = hash_file(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let paths = index.paths_of(&hash).map_err(|e| e.to_string())?;
            let history = index.history(&hash).map_err(|e| e.to_string())?;
            if paths.is_empty() && history.is_empty() {
                return Err(format!("{} is not in the index", file.display()));
            }
            for path in &paths {
                println!("{}", path.display());
            }
            if !history.is_empty() {
                println!();
            }
            for event in history {
                match event.from {
                    Some(from) => println!(
                        "{}\t{}\t{}\tfrom {}",
                        format_utc(event.at),
                        event.kind.as_str(),
                        event.path.display(),
                        from.display()
                    ),
                    None => println!(
                        "{}\t{}\t{}",
                        format_utc(event.at),
                        event.kind.as_str(),
                        event.path.display()
                    ),
                }
            }
            Ok(())
        }
        Command::Inbox { folder } => {
            let folder = match folder {
                Some(folder) => folder,
                None => std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join("Downloads"))
                    .ok_or("name a folder to look at")?,
            };
            for item in index.inbox(&folder).map_err(|e| e.to_string())? {
                let name = item.path.file_name().unwrap_or_default().to_string_lossy();
                match item.advice {
                    Advice::Suggest { folder, reason } => {
                        println!("{name}\t→ {}", folder.display());
                        println!("\t{reason}");
                    }
                    Advice::Known(paths) => {
                        for path in paths {
                            println!("{name}\tis already at {}", path.display());
                        }
                    }
                    Advice::Nothing => println!("{name}\tno suggestion"),
                }
            }
            Ok(())
        }
        Command::Move { files, to, across } => {
            let mut failed = false;
            for file in files {
                match index.move_file(&file, &to, across) {
                    Ok(done) => {
                        println!("moved\t{}\tfrom {}", done.to.display(), done.from.display())
                    }
                    Err(e) => {
                        eprintln!("clerk: {e}");
                        failed = true;
                    }
                }
            }
            match failed {
                true => Err("not every file was moved".to_owned()),
                false => Ok(()),
            }
        }
        Command::File { files, across } => file_all(&mut index, files, across),
        Command::Undo => {
            let undone = index.undo().map_err(|e| e.to_string())?;
            println!(
                "moved back\t{}\tfrom {}",
                undone.from.display(),
                undone.to.display()
            );
            Ok(())
        }
        Command::Journal => {
            for done in index.journal().map_err(|e| e.to_string())? {
                let undone = done
                    .undone
                    .map(|at| format!("\tundone {}", format_utc(at)))
                    .unwrap_or_default();
                println!(
                    "{}\t{}\tfrom {}{undone}",
                    format_utc(done.at),
                    done.to.display(),
                    done.from.display()
                );
            }
            Ok(())
        }
        Command::Watch { verbose } => {
            eprintln!("clerk: watching; stop with Ctrl+C");
            index
                .watch(
                    || false,
                    |scanned| match scanned {
                        Ok(report) => print_report(&report, verbose),
                        Err(e) => eprintln!("clerk: {e}"),
                    },
                )
                .map_err(|e| e.to_string())
        }
        Command::Status => {
            let status = index.status().map_err(|e| e.to_string())?;
            println!("index\t{}", status.index.display());
            println!("files\t{}", status.files);
            println!("documents\t{}", status.documents);
            println!("with text\t{}", status.with_text);
            if status.awaiting_ocr > 0 {
                println!("awaiting OCR\t{}", status.awaiting_ocr);
            }
            for root in status.roots {
                let scanned = root.scanned.map_or("never".to_owned(), format_utc);
                println!(
                    "folder\t{}\t{} files\tscanned {scanned}",
                    root.path.display(),
                    root.files
                );
            }
            Ok(())
        }
    }
}

/// Move each of `files` to where clerk suggests, saying what it did and did not do.
fn file_all(index: &mut Index, files: Vec<PathBuf>, across: bool) -> Result<(), String> {
    let mut failed = false;
    for item in index.advise(&files).map_err(|e| e.to_string())? {
        let path = item.path;
        match item.advice {
            Advice::Suggest { folder, reason } => match index.move_file(&path, &folder, across) {
                Ok(done) => {
                    println!("moved\t{}\tfrom {}", done.to.display(), done.from.display());
                    println!("\t{reason}");
                }
                Err(e) => {
                    eprintln!("clerk: {e}");
                    failed = true;
                }
            },
            Advice::Known(paths) => println!(
                "left\t{}\talready at {}",
                path.display(),
                paths[0].display()
            ),
            Advice::Nothing => println!("left\t{}\tno suggestion", path.display()),
        }
    }
    match failed {
        true => Err("not every file was moved".to_owned()),
        false => Ok(()),
    }
}

fn scan(index: &mut Index, folders: Vec<PathBuf>, verbose: bool) -> Result<(), String> {
    let folders = match folders.is_empty() {
        false => folders,
        true => {
            let known = index.roots().map_err(|e| e.to_string())?;
            match known.is_empty() {
                false => known,
                true => {
                    let home = std::env::var_os("HOME").ok_or("name a folder to scan")?;
                    let documents = PathBuf::from(home).join("Documents");
                    if !documents.is_dir() {
                        return Err("name a folder to scan".to_owned());
                    }
                    vec![documents]
                }
            }
        }
    };
    let live = std::io::stderr().is_terminal();
    for folder in folders {
        let report = index
            .scan(&folder, |progress| {
                if live {
                    match progress {
                        Progress::Files(n) => eprint!("\r\x1b[K{} … {n} files", folder.display()),
                        Progress::Texts(n, of) => {
                            eprint!("\r\x1b[K{} … text {n} of {of}", folder.display())
                        }
                    }
                    let _ = std::io::stderr().flush();
                }
            })
            .map_err(|e| e.to_string())?;
        if live {
            eprint!("\r\x1b[K");
        }
        print_report(&report, verbose);
    }
    Ok(())
}

fn print_report(report: &Report, verbose: bool) {
    println!(
        "{}: {} files, {} read; {} added, {} moved, {} copied, {} changed, {} removed",
        report.root.display(),
        report.files,
        report.hashed,
        report.added.len(),
        report.moved.len(),
        report.copied.len(),
        report.changed.len(),
        report.removed.len()
    );
    let mut texts = format!("  {} with text", report.texts);
    if !report.no_text.is_empty() {
        texts += &format!(", {} whose text could not be read", report.no_text.len());
    }
    if report.awaiting_ocr > 0 {
        texts += &format!(
            ", {} scans or pictures to read once tesseract is installed",
            report.awaiting_ocr
        );
    }
    println!("{texts}");
    if verbose {
        for path in &report.added {
            println!("added\t{}", path.display());
        }
        for (from, to) in &report.moved {
            println!("moved\t{}\tfrom {}", to.display(), from.display());
        }
        for (from, to) in &report.copied {
            println!("copied\t{}\tfrom {}", to.display(), from.display());
        }
        for path in &report.changed {
            println!("changed\t{}", path.display());
        }
        for path in &report.removed {
            println!("removed\t{}", path.display());
        }
        for (path, why) in &report.no_text {
            println!("no text\t{}\t{why}", path.display());
        }
    }
    for (path, why) in &report.unreadable {
        eprintln!("clerk: not read: {}: {why}", path.display());
    }
}
