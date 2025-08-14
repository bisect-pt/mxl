use std::env;
use std::fs;
use std::path::PathBuf;

#[cfg(debug_assertions)]
const BUILD_VARIANT: &str = "Linux-Clang-Debug";
#[cfg(not(debug_assertions))]
const BUILD_VARIANT: &str = "Linux-Clang-Release";

struct BindgenSpecs {
    header: String,
    includes_dirs: Vec<String>,
}

fn get_bindgen_specs() -> BindgenSpecs {
    #[cfg(not(feature = "mxl-not-built"))]
    let header = "wrapper-with-version-h.h".to_string();
    #[cfg(feature = "mxl-not-built")]
    let header = "wrapper-without-version-h.h".to_string();

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("failed to get current directory"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let mut includes_dirs = vec![
        repo_root
            .join("lib")
            .join("include")
            .to_string_lossy()
            .to_string(),
    ];
    if cfg!(not(feature = "mxl-not-built")) {
        let build_dir = repo_root.join("build").join(BUILD_VARIANT);
        let build_version_dir = build_dir
            .join("lib")
            .join("include")
            .to_string_lossy()
            .to_string();

        includes_dirs.push(build_version_dir);
    }

    BindgenSpecs {
        header,
        includes_dirs,
    }
}

fn main() {
    let bindgen_specs = get_bindgen_specs();
    for include_dir in &bindgen_specs.includes_dirs {
        println!("cargo:include={include_dir}");
    }

    if cfg!(not(feature = "mxl-not-built")) {
        // TODO: figure out when this has to be rebuilt
        let mxl_version_out_path =
            PathBuf::from(&env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"))
                .join("mxl");
        if !fs::exists(&mxl_version_out_path).expect("Error checking if out path exists") {
            fs::create_dir(&mxl_version_out_path).expect("Failed to create out path");
        }
        let out_path = mxl_version_out_path.join("version.h");
        println!("cargo:rerun-if-changed={}", out_path.display());

        let dst = cmake::Config::new("../../")
            .define("BUILD_TESTS", "OFF")
            .define("BUILD_TOOLS", "OFF")
            .build();

        let mxl_version_location = dst.join("include").join("mxl").join("version.h");
        assert!(matches!(std::fs::exists(&mxl_version_location), Ok(true)));

        fs::copy(&mxl_version_location, &out_path).expect("Could copy mxl version");

        println!("cargo:rustc-link-search={}", dst.join("lib64").display());
        println!("cargo:rustc-link-lib=mxl");
    }

    let bindings = bindgen::builder()
        .clang_args(
            bindgen_specs
                .includes_dirs
                .iter()
                .map(|dir| format!("-I{dir}")),
        )
        .header(bindgen_specs.header)
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
