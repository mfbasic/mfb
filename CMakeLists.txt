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

add_custom_command(
  OUTPUT "${MFB_EXE}"
  COMMAND "${CMAKE_COMMAND}" -E env
          "CARGO_TARGET_DIR=${CARGO_TARGET_DIR}"
          "${CARGO_EXECUTABLE}" build
          --manifest-path "${CMAKE_SOURCE_DIR}/Cargo.toml"
          ${CARGO_BUILD_FLAGS}
  DEPENDS
          "${CMAKE_SOURCE_DIR}/Cargo.toml"
          "${CMAKE_SOURCE_DIR}/src/main.rs"
          "${CMAKE_SOURCE_DIR}/src/rules.rs"
  WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
  VERBATIM
  USES_TERMINAL
)

add_custom_target(mfb ALL DEPENDS "${MFB_EXE}")

install(PROGRAMS "${MFB_EXE}" DESTINATION bin)
