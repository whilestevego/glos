// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// https://mozilla.org/MPL/2.0/.

//! What `build.rs` compiles and embeds from the pinned Helix release: `GRAMMARS`, `QUERIES`,
//! `THEMES` and `LANGUAGES_TOML`.
include!(concat!(env!("OUT_DIR"), "/embedded.rs"));
