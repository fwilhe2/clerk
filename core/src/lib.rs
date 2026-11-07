// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! clerk's core (`doc/plan.md`): the index, scanning a directory into it, a document's identity
//! by content, and its text for search. A shell calls this and owns nothing.

mod error;
mod index;
mod read;
mod scan;
mod text;
mod time;

pub use error::{Error, Result};
pub use index::{DocumentText, Event, EventKind, Found, Index, RootStatus, Status};
pub use read::{Hash, hash_file};
pub use scan::{Progress, Report};
pub use text::quiet_reader_panics;
pub use time::format_utc;
