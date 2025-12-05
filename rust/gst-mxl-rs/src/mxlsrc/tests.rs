#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use glib::{object::ObjectExt, subclass::types::ObjectSubclassType};
    use gst::{prelude::*, CoreError, ElementFactory, Pipeline};
    use mxl::config::get_mxl_so_path;
    use tracing::trace;

    use crate::mxlsrc::imp::*;

    #[test]
    fn set_properties() -> Result<(), glib::Error> {
        gst::init()?;
        gst::Element::register(None, "mxlsrc", gst::Rank::NONE, MxlSrc::type_())
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let element = gst::ElementFactory::make("mxlsrc")
            .property("video-flow", "test_flow")
            .property("domain", "mydomain")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let flow_id: String = element.property("video-flow");
        let domain: String = element.property("domain");

        assert_eq!(flow_id, "test_flow");
        assert_eq!(domain, "mydomain");
        Ok(())
    }

    #[test]
    //#[ignore]
    #[cfg_attr(feature = "tracing", tracing_test::traced_test)]
    fn negotiate_caps() -> Result<(), glib::Error> {
        gst::init()?;
        gst::Element::register(None, "mxlsrc", gst::Rank::NONE, MxlSrc::type_())
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        let factory = gst::ElementFactory::find("mxlsrc").expect("mxlsrc not registered");
        let pad_templates = factory.static_pad_templates();
        assert!(!pad_templates.is_empty());

        let src_templ = pad_templates
            .iter()
            .find(|t| t.direction() == gst::PadDirection::Src)
            .ok_or(gst::CoreError::Failed)
            .map_err(|_| glib::Error::new(CoreError::Pad, "Pad templates failed"))?;
        trace!("Advertised caps: {}", src_templ.caps());

        let pipeline = gst::Pipeline::new();
        let src = gst::ElementFactory::make("mxlsrc")
            .property("video-flow", "9fbec3b1-1b0f-417d-9059-8b94a47197ed")
            .property("domain", "/mnt/mxl/domain_1")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        let queue1 = gst::ElementFactory::make("queue")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        let convert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        let queue2 = gst::ElementFactory::make("queue")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        let sink = gst::ElementFactory::make("fakesink")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        pipeline
            .add_many(&[&src, &queue1, &convert, &queue2, &sink])
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        gst::Element::link_many([&src, &queue1, &convert, &queue2, &sink])
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| glib::Error::new(CoreError::Failed, "State change failed"))?;

        let src_pad = src
            .static_pad("src")
            .ok_or(CoreError::Failed)
            .map_err(|_| glib::Error::new(CoreError::Pad, "Source pad failed"))?;
        if let Some(caps) = src_pad.current_caps() {
            trace!("Negotiated caps: {}", caps.to_string());
        } else {
            trace!("No negotiated caps found");
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
        pipeline
            .set_state(gst::State::Null)
            .map_err(|_| glib::Error::new(CoreError::Failed, "State change failed"))?;
        Ok(())
    }

    #[test]
    #[ignore]
    #[cfg_attr(feature = "tracing", tracing_test::traced_test)]

    fn start_valid_pipeline() -> Result<(), glib::Error> {
        gst::init()?;
        gst::Element::register(None, "mxlsrc", gst::Rank::NONE, MxlSrc::type_())
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let src = gst::ElementFactory::make("mxlsrc")
            .property("flow-id", "5fbec3b1-1b0f-417d-9059-8b94a47197ed")
            .property("domain", "/mnt/mxl/domain_1")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let sink = gst::ElementFactory::make("fakesink")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let pipeline = gst::Pipeline::new();
        pipeline
            .add_many(&[&src, &sink])
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        src.link(&sink)
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| glib::Error::new(CoreError::Failed, "State change failed"))?;
        thread::sleep(Duration::from_millis(600));
        pipeline
            .set_state(gst::State::Null)
            .map_err(|_| glib::Error::new(CoreError::Failed, "State change failed"))?;
        Ok(())
    }

    #[test]
    #[ignore]
    #[cfg_attr(feature = "tracing", tracing_test::traced_test)]

    fn start_valid_audio_pipeline() -> Result<(), glib::Error> {
        gst::init()?;

        gst::Element::register(None, "rsmxlsrc", gst::Rank::NONE, MxlSrc::type_())
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let src = ElementFactory::make("rsmxlsrc")
            .property("audio-flow", "8fbec3b1-1b0f-417d-9059-8b94a47197ed")
            .property("domain", "/mnt/mxl/domain_1")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let capsfilter = ElementFactory::make("capsfilter")
            .property("caps", &gst::Caps::builder("audio/x-raw").build())
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let queue0 = ElementFactory::make("queue")
            .name("queue0")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let audioconvert = ElementFactory::make("audioconvert")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let queue2 = ElementFactory::make("queue")
            .name("queue2")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let volume = ElementFactory::make("volume")
            .property("volume", 0.1f64)
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let queue3 = ElementFactory::make("queue")
            .name("queue3")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let sink = ElementFactory::make("fakesink")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let pipeline = Pipeline::new();

        pipeline
            .add_many(&[
                &src,
                &capsfilter,
                &queue0,
                &audioconvert,
                &queue2,
                &volume,
                &queue3,
                &sink,
            ])
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        gst::Element::link_many([&audioconvert, &queue2, &volume, &queue3, &sink])
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        gst::Element::link_many([&src, &capsfilter, &queue0])
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        pipeline.set_state(gst::State::Playing).map_err(|_| {
            glib::Error::new(CoreError::Failed, "Failed to set pipeline to Playing")
        })?;

        thread::sleep(Duration::from_secs(1));

        pipeline
            .set_state(gst::State::Null)
            .map_err(|_| glib::Error::new(CoreError::Failed, "Failed to set pipeline to Null"))?;

        Ok(())
    }

    #[test]
    #[ignore]
    #[cfg_attr(feature = "tracing", tracing_test::traced_test)]
    fn is_valid_reader() -> Result<(), glib::Error> {
        gst::init()?;
        gst::Element::register(None, "mxlsrc", gst::Rank::NONE, MxlSrc::type_())
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let element = gst::ElementFactory::make("mxlsrc")
            .property("flow-id", "5fbec3b1-1b0f-417d-9059-8b94a47197ed")
            .property("domain", "/mnt/mxl/domain_1")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let flow_id: String = element.property("flow-id");
        let domain: String = element.property("domain");
        let mxl_api = mxl::load_api(get_mxl_so_path())
            .map_err(|e| glib::Error::new(gst::CoreError::Failed, e.to_string().as_str()))?;

        let mxl_instance = mxl::MxlInstance::new(mxl_api, domain.as_str(), "")
            .map_err(|e| glib::Error::new(gst::CoreError::Failed, e.to_string().as_str()))?;

        let reader = mxl_instance
            .create_flow_reader(flow_id.as_str())
            .map_err(|e| glib::Error::new(gst::CoreError::Failed, e.to_string().as_str()))?;
        assert!(reader.get_info().is_ok());
        Ok(())
    }

    #[test]
    #[ignore]
    #[cfg_attr(feature = "tracing", tracing_test::traced_test)]
    fn full_audio_loop_pipeline() -> Result<(), glib::Error> {
        use gst::prelude::*;
        use gst::{CoreError, ElementFactory, Pipeline};
        use std::thread;
        use std::time::Duration;

        gst::init()?;

        gst::Element::register(None, "rsmxlsrc", gst::Rank::NONE, MxlSrc::type_())
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        gst::Element::register(
            None,
            "rsmxlsink",
            gst::Rank::NONE,
            crate::mxlsink::MxlSink::static_type(),
        )
        .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let flow_id = "8fbec3b1-1b0f-417d-9059-8b94a47197ed";
        let domain = "/mnt/mxl/domain_1";

        let audiotestsrc = ElementFactory::make("audiotestsrc")
            .property("is-live", true)
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let audioconvert1 = ElementFactory::make("audioconvert")
            .name("audioconvert1")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let audioresample = ElementFactory::make("audioresample")
            .name("audioresample")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let rsmxlsink = ElementFactory::make("rsmxlsink")
            .property("flow-id", flow_id)
            .property("domain", domain)
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let queue = ElementFactory::make("queue")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let rsmxlsrc = ElementFactory::make("rsmxlsrc")
            .property("audio-flow", flow_id)
            .property("domain", domain)
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let audioconvert2 = ElementFactory::make("audioconvert")
            .name("audioconvert2")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let autoaudiosink = ElementFactory::make("fakesink")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let pipeline = Pipeline::new();

        pipeline
            .add_many(&[
                &audiotestsrc,
                &audioconvert1,
                &audioresample,
                &rsmxlsink,
                &queue,
                &rsmxlsrc,
                &audioconvert2,
                &autoaudiosink,
            ])
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        gst::Element::link_many([
            &audiotestsrc,
            &audioconvert1,
            &audioresample,
            &rsmxlsink,
            &queue,
            &rsmxlsrc,
            &audioconvert2,
            &autoaudiosink,
        ])
        .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        pipeline.set_state(gst::State::Playing).map_err(|_| {
            glib::Error::new(CoreError::Failed, "Failed to set pipeline to Playing")
        })?;

        thread::sleep(Duration::from_secs(5));

        pipeline
            .set_state(gst::State::Null)
            .map_err(|_| glib::Error::new(CoreError::Failed, "Failed to set pipeline to Null"))?;

        Ok(())
    }
}
