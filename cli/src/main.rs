// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `clerk`: keeps track of the documents in a directory, without ever changing them.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use clerk_core::{Index, Report, format_utc, hash_file};

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
    /// The first scan reads every file, which can take a few minutes; later scans read only
    /// what changed. With no folder, every folder scanned before is scanned again, or
    /// ~/Documents the first time. Hidden files and folders are left out.
    Scan {
        folders: Vec<PathBuf>,
        /// List every file added, moved, copied, changed or removed
        #[arg(long, short)]
        verbose: bool,
    },
    /// Find documents whose path contains every word
    Find {
        #[arg(required = true)]
        words: Vec<String>,
    },
    /// Where a document is now, and where it has been
    ///
    /// The file is recognised by its content, so a copy of it anywhere works too.
    Where { file: PathBuf },
    /// What is indexed, and when it was last scanned
    Status,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
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
        Command::Find { words } => {
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            for found in index.find(&words).map_err(|e| e.to_string())? {
                println!("{}", found.path.display());
            }
            Ok(())
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
        Command::Status => {
            let status = index.status().map_err(|e| e.to_string())?;
            println!("index\t{}", status.index.display());
            println!("files\t{}", status.files);
            println!("documents\t{}", status.documents);
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
            .scan(&folder, |n| {
                if live {
                    eprint!("\r{} … {n} files", folder.display());
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
    }
    for (path, why) in &report.unreadable {
        eprintln!("clerk: not read: {}: {why}", path.display());
    }
}
