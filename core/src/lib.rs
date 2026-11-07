// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! clerk's core (`doc/plan.md`): the index, scanning a directory into it, a document's identity
//! by content, its text for search, and where a new file belongs. A shell calls this and owns nothing.

mod error;
mod index;
mod moves;
mod ocr;
mod read;
mod scan;
mod suggest;
mod text;
mod time;
mod watch;

pub use error::{Error, Result};
pub use index::{DocumentText, Event, EventKind, Found, Index, RootStatus, Status};
pub use moves::Move;
pub use read::{Hash, hash_file};
pub use scan::{Progress, Report};
pub use suggest::{Advice, InboxItem, Reason, template};
pub use text::quiet_reader_panics;
pub use time::format_utc;
