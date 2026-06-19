#!/bin/bash

# SPDX-FileCopyrightText: 2025 Contributors to the Media eXchange Layer project.
# SPDX-License-Identifier: Apache-2.0

set -e

cd "$(dirname "$0")/.."

OUTPUT_DIR="${1:-debs}"
PRESET="${2:-Linux-GCC-Release}"
UBUNTU_VERSION=$(lsb_release -rs 2>/dev/null || echo "unknown")

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
for f in "build/${PRESET}"/*.deb; do
    [ -f "$f" ] && cp "$f" "${OUTPUT_DIR}/$(basename "${f}" .deb)-ubuntu${UBUNTU_VERSION}.deb"
done

pushd rust
    if ! command -v cargo-deb &>/dev/null; then
        echo "Installing cargo-deb..."
        cargo install cargo-deb
    fi
    cargo deb -p gst-mxl-rs

    for f in target/debian/*.deb; do
        [ -f "$f" ] && cp "$f" "../${OUTPUT_DIR}/$(basename "${f}" .deb)-ubuntu${UBUNTU_VERSION}.deb"
    done
popd

mkdir -p $OUTPUT_DIR/dmfmxl-rs-bindings/DEBIAN
mkdir -p $OUTPUT_DIR/dmfmxl-rs-bindings/usr/include
mkdir -p $OUTPUT_DIR/dmfmxl-rs-bindings/opt/bisect/

cp -r rust/mxl-sys/ $OUTPUT_DIR/dmfmxl-rs-bindings/opt/bisect/
cp -r rust/mxl/ $OUTPUT_DIR/dmfmxl-rs-bindings/opt/bisect/

CONTROL_FILE_CONTENT="Package: dmfmxl-rs-bindings
Version: 1.0
Architecture: all
Maintainer: Bisect Lda. <info@bisect.pt>
Description: MXL rust bindings."

touch $OUTPUT_DIR/dmfmxl-rs-bindings/DEBIAN/control
echo "$CONTROL_FILE_CONTENT" > $OUTPUT_DIR/dmfmxl-rs-bindings/DEBIAN/control

dpkg-deb --build "${OUTPUT_DIR}/dmfmxl-rs-bindings" "${OUTPUT_DIR}/dmfmxl-rs-bindings-ubuntu${UBUNTU_VERSION}.deb"

rm -rf $OUTPUT_DIR/dmfmxl-rs-bindings

echo "Release package(s) copied to ${OUTPUT_DIR}/"
