// Copyright (C) 2020 Sebastian Dröge <sebastian@centricular.com>
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use gst::glib;
use gst::prelude::*;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::util::SubscriberInitExt;

mod imp;

glib::wrapper! {
    pub struct MxlSrc(ObjectSubclass<imp::MxlSrc>) @extends gst_base::PushSrc, gst_base::BaseSrc, gst::Element, gst::Object;
}

pub fn register(plugin: &gst::Plugin) -> Result<(), glib::BoolError> {
    let _ = tracing_subscriber::fmt()
        .compact()
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(true)
        .with_target(false)
        .with_max_level(LevelFilter::TRACE)
        .with_ansi(true)
        .finish()
        .try_init();
    gst::Element::register(
        Some(plugin),
        "rsmxlsrc",
        gst::Rank::NONE,
        MxlSrc::static_type(),
    )
}
