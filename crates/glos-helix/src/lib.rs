// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// https://mozilla.org/MPL/2.0/.

//! Everything glos takes from Helix 25.07.1, in one MPL-2.0 crate: glos itself never names
//! `helix-core`, `helix-stdx` or `tree-house`.
//!
//! - [`text`] and [`layout`]: the parts of `helix-core` glos uses, re-exported. Using something
//!   new from Helix means adding it here, so this is the list to check on a Helix upgrade.
//! - [`commands`]: Helix's commands, ported from `helix-term` onto `helix-core`.
//! - [`syntax`]: highlighting with `tree-house`, Helix's `languages.toml` and queries, and the
//!   grammars compiled in by `build.rs`.
//! - [`THEMES`]: Helix's themes, as text.

pub mod commands;
mod embedded;
pub mod syntax;

/// Helix's themes: (name, TOML text).
pub use embedded::THEMES;

/// Text, selections and edits.
pub mod text {
    pub use helix_core::{
        Assoc, Position, Range, Rope, RopeSlice, Selection, Tendril, Transaction,
        diff::compare_ropes,
        graphemes::{Grapheme, next_grapheme_boundary, prev_grapheme_boundary},
        line_ending::line_end_char_index,
    };
    pub use helix_stdx::rope::RopeSliceExt;
}

/// Laying text out on screen: soft wrap, tabs, wide characters and virtual lines.
pub mod layout {
    pub use helix_core::{
        char_idx_at_visual_offset,
        doc_formatter::{DocumentFormatter, TextFormat},
        text_annotations::{LineAnnotation, TextAnnotations},
        visual_offset_from_anchor,
    };
}

/// Helix's selection notation for tests: `#[h|]#ello`.
pub mod test {
    pub use helix_core::test::{plain, print};
}
