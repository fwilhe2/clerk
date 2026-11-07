// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! C2's guard (`doc/plan.md`): nothing clerk builds talks to another computer — no client crate
//! in the lockfile, no socket or spawned downloader in the code.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Crates whose job is talking to another computer: HTTP and WebSocket clients, TLS stacks.
const CLIENTS: &[&str] = &[
    "attohttpc",
    "curl",
    "curl-sys",
    "h2",
    "hyper",
    "isahc",
    "minreq",
    "native-tls",
    "openssl",
    "reqwest",
    "rustls",
    "surf",
    "tungstenite",
    "ureq",
    "webpki-roots",
];

#[test]
fn no_network_client_is_in_the_lockfile() {
    let lock = std::fs::read_to_string(root().join("Cargo.lock")).expect("Cargo.lock");
    let names: Vec<&str> = lock
        .lines()
        .filter_map(|line| line.strip_prefix("name = \""))
        .filter_map(|rest| rest.strip_suffix('"'))
        .collect();
    assert!(names.contains(&"rusqlite"), "read {} packages", names.len());
    let found: Vec<&&str> = names.iter().filter(|n| CLIENTS.contains(n)).collect();
    assert!(found.is_empty(), "network crates in Cargo.lock: {found:?}");
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "tests" {
                sources(&path, out);
            }
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_code_opens_a_socket_or_runs_a_downloader() {
    let mut all = Vec::new();
    sources(&root(), &mut all);
    assert!(all.len() >= 5, "found {} files", all.len());
    let words = [
        "std::net",
        "TcpStream",
        "UdpSocket",
        "Command::new(\"curl\")",
        "Command::new(\"wget\")",
    ];
    let offences: Vec<String> = all
        .iter()
        .flat_map(|path| {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            words
                .iter()
                .filter(move |w| text.contains(**w))
                .map(move |w| format!("{}: {w}", path.display()))
        })
        .collect();
    assert!(offences.is_empty(), "{offences:#?}");
}
