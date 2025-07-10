/// Tests of the basic low level synchronous API.
///
/// The tests now require an MXL library of a specific name to be present in the system. This should
/// change in the future. For now, feel free to just edit the path to your library.
use std::time::Duration;

use mxl::{OwnedGrainData, config::get_mxf_so_path};
use tracing::info;

static LOG_ONCE: std::sync::Once = std::sync::Once::new();

fn setup_empty_domain() -> String {
    // TODO: Randomize the domain name to allow parallel tests run?
    //       On the other hand, some tests may leave stuff behind, this way we do not keep
    //       increasing garbage from the tests.
    let result = "/dev/shm/mxl_rust_unit_tests_domain";
    if std::path::Path::new(result).exists() {
        std::fs::remove_dir_all(result).expect("Failed to remove existing test domain directory");
    }
    std::fs::create_dir_all(result).expect("Failed to create test domain directory");
    result.to_string()
}

fn setup_test() -> mxl::MxlInstance {
    // Set up the logging to use the RUST_LOG environment variable and if not present, print INFO
    // and higher.
    LOG_ONCE.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::builder()
                    .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
                    .from_env_lossy(),
            )
            .init();
    });

    let mxl_api = mxl::load_api(get_mxf_so_path()).unwrap();
    let domain = setup_empty_domain();
    mxl::MxlInstance::new(mxl_api, &domain, "").unwrap()
}

#[test]
fn basic_mxl_writing_reading() {
    let mxl_instance = setup_test();
    let flow_config_file = mxl::config::get_mxl_repo_root().join("lib/tests/data/v210_flow.json");
    let flow_def = mxl::tools::read_file(flow_config_file.as_path())
        .map_err(|error| {
            mxl::Error::Other(format!(
                "Error while reading flow definition from \"{}\": {}",
                flow_config_file.display(),
                error
            ))
        })
        .unwrap();
    let flow_info = mxl_instance.create_flow(flow_def.as_str(), None).unwrap();
    let flow_id = flow_info.common_flow_info().id().to_string();
    let flow_writer = mxl_instance.create_flow_writer(flow_id.as_str()).unwrap();
    let flow_reader = mxl_instance.create_flow_reader(flow_id.as_str()).unwrap();
    let rate = flow_info.discrete_flow_info().unwrap().grainRate;
    let current_index = mxl_instance.get_current_index(&rate);
    let grain_write_access = flow_writer.open_grain(current_index).unwrap();
    let grain_size = grain_write_access.max_size();
    grain_write_access.commit(grain_size).unwrap();
    let grain_data = flow_reader
        .get_complete_grain(current_index, Duration::from_secs(5))
        .unwrap();
    let grain_data: OwnedGrainData = grain_data.into();
    info!("Grain data len: {:?}", grain_data.payload.len());
    flow_reader.destroy().unwrap();
    flow_writer.destroy().unwrap();
    mxl_instance.destroy_flow(flow_id.as_str()).unwrap();
    unsafe { mxl_instance.destroy() }.unwrap();
}
