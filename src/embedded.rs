//! What `build.rs` compiles and embeds from the pinned Helix release: `GRAMMARS`, `QUERIES`,
//! `THEMES` and `LANGUAGES_TOML`.
include!(concat!(env!("OUT_DIR"), "/embedded.rs"));
