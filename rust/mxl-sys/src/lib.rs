#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(missing_docs)]
#![allow(rustdoc::broken_intra_doc_links)]
#![allow(rustdoc::invalid_html_tags)]
// Suppress expected warnings from bindgen-generated code.
// See https://github.com/rust-lang/rust-bindgen/issues/1651.
#![allow(deref_nullptr)]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
