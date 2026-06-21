cmake_minimum_required(VERSION 3.20)

project(mfb LANGUAGES NONE)

find_program(CARGO_EXECUTABLE cargo REQUIRED)

set(CARGO_TARGET_DIR "${CMAKE_BINARY_DIR}/cargo-target")

if(CMAKE_BUILD_TYPE STREQUAL "Release")
  set(CARGO_BUILD_FLAGS --release)
  set(CARGO_PROFILE_DIR release)
else()
  set(CARGO_BUILD_FLAGS)
  set(CARGO_PROFILE_DIR debug)
endif()

if(WIN32)
  set(MFB_EXE "${CARGO_TARGET_DIR}/${CARGO_PROFILE_DIR}/mfb.exe")
else()
  set(MFB_EXE "${CARGO_TARGET_DIR}/${CARGO_PROFILE_DIR}/mfb")
endif()

# Track every Rust source under src/ (and the manifests) so the cargo build
# re-runs when any of them change. A recursive glob with CONFIGURE_DEPENDS keeps
# this list current automatically — the previous hand-maintained list drifted
# (it named the deleted src/bytecode.rs and omitted newer modules), which made
# `cmake --build` fail with "No rule to make target ... src/bytecode.rs".
file(GLOB_RECURSE MFB_RUST_SOURCES CONFIGURE_DEPENDS
  "${CMAKE_SOURCE_DIR}/src/*.rs"
)

add_custom_command(
  OUTPUT "${MFB_EXE}"
  COMMAND "${CMAKE_COMMAND}" -E env
          "CARGO_TARGET_DIR=${CARGO_TARGET_DIR}"
          "${CARGO_EXECUTABLE}" build
          --manifest-path "${CMAKE_SOURCE_DIR}/Cargo.toml"
          ${CARGO_BUILD_FLAGS}
  DEPENDS
          "${CMAKE_SOURCE_DIR}/Cargo.toml"
          ${MFB_RUST_SOURCES}
  WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
  VERBATIM
  USES_TERMINAL
)

add_custom_target(mfb ALL DEPENDS "${MFB_EXE}")

add_custom_target(
  test-accept
  COMMAND "${CMAKE_SOURCE_DIR}/scripts/test-accept.sh"
          "${MFB_EXE}"
          "${CMAKE_BINARY_DIR}/accept-actual"
  DEPENDS mfb
  WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
  VERBATIM
  USES_TERMINAL
)

install(PROGRAMS "${MFB_EXE}" DESTINATION bin)
