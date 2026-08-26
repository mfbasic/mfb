#!/usr/bin/env bash
set -e

# Setup working directories
ROOT_DIR="$(pwd)/build"
BUILD_DIR="$ROOT_DIR/deps"
PREFIX_DIR="$ROOT_DIR/install"
OUTPUT_DIR="$ROOT_DIR/output"

mkdir -p "$BUILD_DIR" "$PREFIX_DIR" "$OUTPUT_DIR"

export CFLAGS="-fPIC -O2"
export CXXFLAGS="-fPIC -O2"

# Prefer clang: some boxes pair a new gcc with an old binutils, and gcc then
# emits .base64 pseudo-ops the system assembler rejects; clang's integrated
# assembler avoids the mismatch. CC/CXX in the environment still win.
export CC="${CC:-$(command -v clang || command -v cc)}"
export CXX="${CXX:-$(command -v clang++ || command -v c++)}"
export PKG_CONFIG_PATH="$PREFIX_DIR/lib/pkgconfig"

# Keep system/homebrew copies of the codecs out of every find_* lookup so we
# only ever link the static libs built into $PREFIX_DIR.
IGNORE_PREFIXES="/usr;/usr/local;/opt/homebrew;$HOME/local/brew"

# 1. Build Ogg
cd "$BUILD_DIR"
[ -d ogg ] || git clone --depth 1 --branch v1.3.6 https://github.com/xiph/ogg.git
cd ogg && cmake -B build -DBUILD_SHARED_LIBS=OFF -DCMAKE_INSTALL_PREFIX="$PREFIX_DIR" -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DCMAKE_IGNORE_PREFIX_PATH="$IGNORE_PREFIXES" -DCMAKE_POLICY_VERSION_MINIMUM=3.5
cmake --build build --target install

# 2. Build Vorbis
cd "$BUILD_DIR"
[ -d vorbis ] || git clone --depth 1 --branch v1.3.7 https://github.com/xiph/vorbis.git
cd vorbis && cmake -B build -DBUILD_SHARED_LIBS=OFF -DCMAKE_INSTALL_PREFIX="$PREFIX_DIR" -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DOGG_ROOT="$PREFIX_DIR" -DCMAKE_IGNORE_PREFIX_PATH="$IGNORE_PREFIXES" -DCMAKE_POLICY_VERSION_MINIMUM=3.5
cmake --build build --target install

# 3. Build FLAC
cd "$BUILD_DIR"
[ -d flac ] || git clone --depth 1 --branch 1.5.0 https://github.com/xiph/flac.git
cd flac && cmake -B build -DBUILD_SHARED_LIBS=OFF -DBUILD_CXX=OFF -DBUILD_PROGRAMS=OFF -DBUILD_EXAMPLES=OFF -DINSTALL_MANPAGES=OFF -DCMAKE_INSTALL_PREFIX="$PREFIX_DIR" -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DOGG_ROOT="$PREFIX_DIR" -DCMAKE_IGNORE_PREFIX_PATH="$IGNORE_PREFIXES" -DCMAKE_POLICY_VERSION_MINIMUM=3.5
cmake --build build --target install

# 4. Build Opus
cd "$BUILD_DIR"
[ -d opus ] || git clone --depth 1 --branch v1.5.2 https://github.com/xiph/opus.git
cd opus && cmake -B build -DBUILD_SHARED_LIBS=OFF -DCMAKE_INSTALL_PREFIX="$PREFIX_DIR" -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DCMAKE_IGNORE_PREFIX_PATH="$IGNORE_PREFIXES" -DCMAKE_POLICY_VERSION_MINIMUM=3.5
cmake --build build --target install

# 5. Build libsndfile using the newly built static dependencies
cd "$BUILD_DIR"
[ -d libsndfile ] || git clone --depth 1 --branch 1.2.2 https://github.com/libsndfile/libsndfile.git
cd libsndfile

cmake -B build \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=ON \
  -DBUILD_PROGRAMS=OFF \
  -DBUILD_EXAMPLES=OFF \
  -DBUILD_TESTING=OFF \
  -DBUILD_XIPH_LIBS=ON \
  -DCMAKE_PREFIX_PATH="$PREFIX_DIR" \
  -DCMAKE_FIND_LIBRARY_SUFFIXES=".a" \
  -DCMAKE_IGNORE_PREFIX_PATH="$IGNORE_PREFIXES" \
  -DCMAKE_INSTALL_PREFIX="$OUTPUT_DIR" \
  -DCMAKE_POLICY_VERSION_MINIMUM=3.5

cmake --build build --target install

# 6. Show the built library's dynamic dependencies.
if [ "$(uname -s)" = "Darwin" ]; then
  otool -L "$OUTPUT_DIR/lib/libsndfile.dylib"
else
  ldd "$OUTPUT_DIR/lib/libsndfile.so"
fi

echo ""
echo "=== Done ==="
echo "libsndfile library installed to: $OUTPUT_DIR"