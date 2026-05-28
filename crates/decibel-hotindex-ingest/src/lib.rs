//! Ingest and parser adapter crate for Decibel HotIndex.

pub mod decibel_parser;

pub use decibel_parser::{
    parse_fixture_jsonl_file, parse_fixture_jsonl_str, ParserOptions, ParserOutput,
};

pub fn crate_status() -> &'static str {
    decibel_hotindex_core::crate_status()
}
