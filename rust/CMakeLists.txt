# Build Rust code in the 'rust' directory using cargo
add_custom_target(rust_build ALL
    COMMAND cargo build --manifest-path=${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml
    WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
    COMMENT "Building Rust code with cargo"
    VERBATIM
)
