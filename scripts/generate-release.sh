#!/bin/bash

# SPDX-FileCopyrightText: 2025 Contributors to the Media eXchange Layer project.
# SPDX-License-Identifier: Apache-2.0

set -e

cd "$(dirname "$0")/.."

OUTPUT_DIR="${1:-debs}"
PRESET="${2:-Linux-GCC-Release}"

VCPKG_TOOLCHAIN="${VCPKG_ROOT:+${VCPKG_ROOT}/scripts/buildsystems/vcpkg.cmake}"
if [ -z "${VCPKG_TOOLCHAIN}" ]; then
    VCPKG_TOOLCHAIN="${HOME}/vcpkg/scripts/buildsystems/vcpkg.cmake"
fi

if [ ! -f "${VCPKG_TOOLCHAIN}" ]; then
    echo "Error: vcpkg toolchain file not found at '${VCPKG_TOOLCHAIN}'." >&2
    echo "Install vcpkg or set VCPKG_ROOT to the vcpkg directory." >&2
    exit 1
fi

mkdir -p build

pushd build
    cmake .. --preset "${PRESET}" \
        -DCMAKE_TOOLCHAIN_FILE="${VCPKG_TOOLCHAIN}" \
        -DMXL_ENABLE_FABRICS_OFI=ON
    rm -f "${PRESET}/*.deb"
    cmake --build "${PRESET}" -j "$(nproc)" -t all doc install package
popd

mkdir -p ${OUTPUT_DIR}
cp "build/${PRESET}"/*.deb "${OUTPUT_DIR}/" 2>/dev/null

pushd rust
    if ! command -v cargo-deb &>/dev/null; then
        echo "Installing cargo-deb..."
        cargo install cargo-deb
    fi
    cargo deb -p gst-mxl-rs

    cp target/debian/*.deb "../${OUTPUT_DIR}/" 2>/dev/null || true
popd

mkdir -p $OUTPUT_DIR/mxl-rs-bindings/DEBIAN
mkdir -p $OUTPUT_DIR/mxl-rs-bindings/usr/include
mkdir -p $OUTPUT_DIR/mxl-rs-bindings/opt/bisect/

cp -r rust/mxl-sys/ $OUTPUT_DIR/mxl-rs-bindings/opt/bisect/
cp -r rust/mxl/ $OUTPUT_DIR/mxl-rs-bindings/opt/bisect/

CONTROL_FILE_CONTENT="Package: mxl-rs-bindings
Version: 1.0
Architecture: all
Maintainer: Bisect Lda. <info@bisect.pt>
Description: MXL rust bindings."

touch $OUTPUT_DIR/mxl-rs-bindings/DEBIAN/control
echo "$CONTROL_FILE_CONTENT" > $OUTPUT_DIR/mxl-rs-bindings/DEBIAN/control

dpkg-deb --build $OUTPUT_DIR/mxl-rs-bindings

rm -rf $OUTPUT_DIR/mxl-rs-bindings

echo "Release package(s) copied to ${OUTPUT_DIR}/"
