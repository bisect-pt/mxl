mod common;

use clap::Parser;
use mxl::config::get_mxf_so_path;
use tracing::info;

#[derive(Debug, Parser)]
#[command(version = clap::crate_version!(), author = clap::crate_authors!())]
pub struct Opts {
    /// The path to the shmem directory where the mxl domain is mapped.
    #[arg(long)]
    pub mxl_domain: String,

    /// The path to the configuration file describing the flow we want to write to.
    #[arg(long)]
    pub flow_config_file: String,

    /// The number of grains to write.
    #[arg(long)]
    pub grain_count: Option<u64>,
}

fn main() -> Result<(), mxl::Error> {
    common::setup_logging();
    let opts: Opts = Opts::parse();

    let mxl_api = mxl::load_api(get_mxf_so_path())?;
    let mxl_instance = mxl::MxlInstance::new(mxl_api, &opts.mxl_domain, "")?;
    let flow_def = mxl::tools::read_file(opts.flow_config_file.as_str()).map_err(|error| {
        mxl::Error::Other(format!(
            "Error while reading flow definition from \"{}\": {}",
            &opts.flow_config_file, error
        ))
    })?;
    let flow_info = mxl_instance.create_flow(flow_def.as_str(), None)?;
    let flow_id = flow_info.common_flow_info().id().to_string();
    let grain_rate = flow_info.discrete_flow_info()?.grainRate;
    let mut grain_index = mxl_instance.get_current_index(&grain_rate);
    info!(
        "Will write to flow \"{flow_id}\" with grain rate {}/{} starting from index {grain_index}.",
        grain_rate.numerator, grain_rate.denominator
    );
    let writer = mxl_instance.create_flow_writer(flow_id.as_str())?;

    let mut remaining_grains = opts.grain_count;
    loop {
        if let Some(count) = remaining_grains {
            if count == 0 {
                break;
            }
            remaining_grains = Some(count - 1);
        }

        let mut grain_writer = writer.open_grain(grain_index)?;
        let payload = grain_writer.payload_mut();
        let payload_len = payload.len();
        for i in 0..payload_len {
            payload[i] = ((i as u64 + grain_index) % 256) as u8;
        }
        grain_writer.commit(payload_len as u32)?;

        let timestamp = mxl_instance.index_to_timestamp(grain_index + 1, &grain_rate)?;
        let sleep_duration = mxl_instance.get_duration_until_index(grain_index + 1, &grain_rate)?;
        info!(
            "Finished writing {payload_len} bytes into grain {grain_index}, will sleep for {:?} until timestamp {timestamp}.",
            sleep_duration
        );
        grain_index += 1;
        mxl_instance.sleep_for(sleep_duration);
    }

    info!("Finished writing requested number of grains, deleting the flow.");
    writer.destroy()?;
    mxl_instance.destroy_flow(flow_id.as_str())?;
    Ok(())
}
