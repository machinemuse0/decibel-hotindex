//! Ingest and parser adapter crate for Decibel HotIndex.

pub mod decibel_parser;

pub use decibel_parser::{
    parse_decibel_event_from_parts, parse_fixture_jsonl_file, parse_fixture_jsonl_str,
    DecibelEventInput, ParserOptions, ParserOutput,
};

pub fn crate_status() -> &'static str {
    decibel_hotindex_core::crate_status()
}
