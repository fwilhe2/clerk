<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# clerk

A local app that helps keep a documents directory in order: it indexes what is there, finds a
document by its name, its metadata or its text, keeps track of a document wherever it is moved,
and suggests where a new download belongs. Something like paperless, but an app rather than a
server, with no setup, and a guest in the directory rather than its owner.

## The requirements

These are normative. Each has a test that holds it.

- **C1 — A guest, never the owner.** clerk never creates, changes, renames, moves or deletes
  anything in a directory it indexes, unless the person asks for that one change. It does not
  leave a dotfile, a sidecar or a lock. Documents are opened read-only; on Linux without
  updating their access time where the system allows. Held by `core/tests/guest.rs`.
- **C2 — Local only.** clerk talks to no other computer: no service, no update check, no
  telemetry, not as an option. OCR and text extraction run on this machine. Held by
  `cli/tests/local_only.rs`.
- **C3 — No setup.** `clerk scan ~/Documents` is the whole setup. There is no configuration to
  write first, no account and no server. What clerk learns, it learns from the directory as the
  person already keeps it.
- **C4 — The index is disposable, and safe.** One SQLite file in `$XDG_DATA_HOME/clerk/`,
  written in transactions. Deleting it loses nothing that was in the documents directory. What
  the person adds that only the index holds (tags, notes, rules) can be exported as plain text.
- **C5 — A move is the person's.** A suggestion is only a suggestion. A move the person accepts
  never overwrites (`renameat2(RENAME_NOREPLACE)` on Linux), never crosses file systems without
  asking, and goes into a journal that can undo it.
- **C6 — Every suggestion explains itself.** "47 files named `Kontoauszug_<n>_<date>.pdf` are in
  `Finanzen/Bank A/`" rather than a score.

## Identity

A document is its **content**: the BLAKE3 hash of its bytes. A path is where a document is now.
That is what lets clerk follow a document that was moved, renamed or copied, by hand or by
anything else, and attach what it knows (text, tags, history) to the document rather than to a
path that changes.

A rescan skips hashing a file whose size and modification time are unchanged. A changed file is a
new document at the same path, and history says so.

## Architecture

Core and shells, as in grind: `clerk-core` holds the index, the scanner and later the
extraction and the suggestions; the `clerk` CLI and any later GUI only call it. Whatever a GUI
can do, the CLI can do.

| Crate | Directory | Holds |
|---|---|---|
| `clerk-core` | `core/` | The index (SQLite), scanning, identity, history, search |
| `clerk-cli` | `cli/` | The `clerk` binary |

## Depending on grind

Office documents are read by [grind](https://github.com/fwilhe2/grind)'s readers, the same ones
its word processor and spreadsheet use: `grind-open` tells ODF, `.docx` and `.xlsx` apart by
their bytes and hands back a document. Both projects are AGPL-3.0-or-later.

- **A git dependency on `main`**, since grind has no release:
  `grind-open = { git = "https://github.com/fwilhe2/grind", branch = "main" }`. Nothing is
  published; grind's crates are `0.0.0` and cargo finds them by name in grind's workspace.
- **`Cargo.lock` pins the commit.** clerk moves to grind's latest `main` only on
  `cargo update -p grind-open`, run on purpose and followed by the tests.
- **Working on both at once:** a `.cargo/config.toml` that is never committed (`.gitignore`
  names it) points clerk at the checkout next to it:

  ```toml
  [patch."https://github.com/fwilhe2/grind"]
  grind-open = { path = "../grind/open" }
  grind-core = { path = "../grind/core" }
  grind-sheet = { path = "../grind/sheet" }
  grind-text = { path = "../grind/text" }
  ```

  The change lands in grind first; clerk then updates to it without the patch.
- **C2 still holds:** cargo fetches grind when building, and grind's own guard (its R12) keeps
  network crates out of it, which `cli/tests/local_only.rs` checks again in clerk's lockfile.
- **What clerk calls:** `grind_open::text(name, bytes)`, all of a document's text in reading
  order, for anything grind opens. Getting it from viewports would be clerk working around grind,
  so it lives in grind, beside `grind_text::Document::text` and `grind_sheet::Document::text`.

## Milestones

| | What | Done when |
|---|---|---|
| **M0** | Workspace, this plan, C1 and C2 guards | `cargo test` holds both |
| **M1** | `clerk scan`, `find` by name, `where`, `status`; moves, copies and changes recorded | A moved file keeps its history |
| **M2** | Text: PDFs with a text layer, ODF and OOXML through grind's readers (a whole-text call added to grind first), plain text; full-text search (FTS5) | `clerk find` finds a word inside a statement |
| **M3** | OCR for scans, through the system's `tesseract`, into the index only | A scanned letter is found by a word in it |
| **M4** | Suggestions: filename templates learned from where files already are; `clerk inbox ~/Downloads` | A bank statement in Downloads is offered its folder, with the reason |
| **M5** | Moves: accept a suggestion, the journal, `clerk undo` | A move can be undone, and never overwrites |
| **M6** | Watching while open (inotify), a GUI | — |

## Not doing

- A server, a daemon or a web interface.
- Writing into documents: no OCR layer added to a PDF, no metadata written into a file.
- Anything that needs another computer, however useful (C2).
