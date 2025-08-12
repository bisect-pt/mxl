// SPDX-FileCopyrightText: 2025 2025 Contributors to the Media eXchange Layer project.
// SPDX-License-Identifier: Apache-2.0

use std::env;
use std::path::PathBuf;

#[cfg(not(feature = "mxl-not-built"))]
#[cfg(debug_assertions)]
const BUILD_VARIANT: &str = "Linux-Clang-Debug";
#[cfg(not(feature = "mxl-not-built"))]
#[cfg(not(debug_assertions))]
const BUILD_VARIANT: &str = "Linux-Clang-Release";

struct BindgenSpecs {
    header: String,
    includes_dirs: Vec<String>,
}

fn get_includes_dirs() -> Vec<String> {
    if let Ok(install_root) = env::var("MXL_INSTALL_ROOT").map(PathBuf::from) {
        return vec![install_root.join("include").to_string_lossy().to_string()];
    }

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("failed to get current directory"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap().to_owned();
    let lib_include_dir = repo_root
        .join("lib")
        .join("include")
        .to_string_lossy()
        .to_string();

    #[cfg(feature = "mxl-not-built")]
    {
        vec![lib_include_dir]
    }

    #[cfg(not(feature = "mxl-not-built"))]
    {
        let build_dir = repo_root.join("build").join(BUILD_VARIANT);
        let build_version_dir = build_dir
            .join("lib")
            .join("include")
            .to_string_lossy()
            .to_string();

        vec![lib_include_dir, build_version_dir]
    }
}

fn get_bindgen_specs() -> BindgenSpecs {
    #[cfg(not(feature = "mxl-not-built"))]
    let header = "wrapper-with-version-h.h".to_string();
    #[cfg(feature = "mxl-not-built")]
    let header = "wrapper-without-version-h.h".to_string();

    let includes_dirs = get_includes_dirs();

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
