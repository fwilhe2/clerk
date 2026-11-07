// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! clerk's core (`doc/plan.md`): the index, scanning a directory into it, and a document's
//! identity by content. A shell calls this and owns nothing.

mod error;
mod index;
mod read;
mod scan;
mod time;

pub use error::{Error, Result};
pub use index::{Event, EventKind, Found, Index, RootStatus, Status};
pub use read::{Hash, hash_file};
pub use scan::Report;
pub use time::format_utc;
