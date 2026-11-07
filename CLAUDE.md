# CLAUDE.md

Guidance for Claude Code in this repo. `doc/plan.md` is the design and the milestones; read it
before changing anything.

## What this is

`clerk`: a local app that indexes a documents folder, finds documents by name, metadata and
(later) text, follows a document wherever it is moved, and suggests where a download belongs.
Like paperless, but an app, with no setup, and a guest in the folder rather than its owner.

## The rules that govern everything

`doc/plan.md`'s requirements are normative; each has a test.

- **C1 — a guest.** Nothing clerk does unasked creates, changes, moves or deletes anything in an
  indexed folder: no dotfile, no sidecar, no lock. Documents are opened read-only
  (`core/src/read.rs`). The index lives outside (`$XDG_DATA_HOME/clerk/`), and a scan refuses a
  folder that contains it. `core/tests/guest.rs` snapshots a folder before and after.
- **C2 — local only.** No network, not even as an option. `cli/tests/local_only.rs`.
- **C3 — no setup.** `clerk scan ~/Documents` is the setup.
- **C5 — a move is the person's**, never overwrites, and can be undone.

A document is its content (BLAKE3); a path is where it is now.

## Commands

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
RUSTDOCFLAGS=-D warnings cargo doc --no-deps
cargo run -- --index /tmp/i.sqlite scan ~/Documents
```

Use `--index` (or `CLERK_INDEX`) when trying it, so the person's own index is not touched.

## Architecture

| Crate | Directory | Holds |
|---|---|---|
| `clerk-core` | `core/` | The index (SQLite, migrations by `user_version`), scanning, identity, history, search |
| `clerk-cli` | `cli/` | The `clerk` binary; calls the core and owns nothing |

Office documents are read through grind's crates, a git dependency on grind's `main`
(`doc/plan.md`, "Depending on grind"). A change both need lands in grind first; to build against
the checkout at `../grind` meanwhile, use the uncommitted `.cargo/config.toml` patch described
there, and remove it before committing.

A schema change is a new entry in `MIGRATIONS` (`core/src/index.rs`), never an edit of an old
one: an index on disk was made by the old one.

## Conventions

- Comments say what the code and its names cannot; no history, milestones or status in code.
- `ponytail:` marks a deliberate shortcut with a known ceiling.
- clap `///` text is `--help` output: written for the person, no doc references.
- Never use the person's name, email or files as test data or examples.

## Licensing

REUSE: code AGPL-3.0-or-later, prose in `doc/` CC-BY-4.0, generated or trivial files CC0-1.0 in
`REUSE.toml`.
