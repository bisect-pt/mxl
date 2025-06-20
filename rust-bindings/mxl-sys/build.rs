use std::env;
use std::path::PathBuf;

fn main() {
    let headers_dir = "mxl-headers-2025-06-17";
    let headers = [
        "mxl/dataformat.h",
        "mxl/flow.h",
        "mxl/flowinfo.h",
        "mxl/mxl.h",
        "mxl/platform.h",
        "mxl/rational.h",
        "mxl/time.h",
        "mxl/version.h",
    ];

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("failed to get current directory");
    let includes_dir = format!("{manifest_dir}/{headers_dir}");
    println!("cargo:include={includes_dir}");

    let bindings = bindgen::builder()
        .clang_arg(format!("-I{includes_dir}"))
        .headers(
            headers
                .iter()
                .map(|val| format!("{includes_dir}/{val}"))
                .collect::<Vec<_>>(),
        )
        .derive_default(true)
        .derive_debug(true)
        .prepend_enum_name(false)
        .generate()
        .unwrap();

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Could not write bindings");
}
