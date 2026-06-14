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
          "${CMAKE_SOURCE_DIR}/src/arch/mod.rs"
          "${CMAKE_SOURCE_DIR}/src/arch/aarch64/abi.rs"
          "${CMAKE_SOURCE_DIR}/src/arch/aarch64/encode.rs"
          "${CMAKE_SOURCE_DIR}/src/arch/aarch64/mod.rs"
          "${CMAKE_SOURCE_DIR}/src/arch/aarch64/ops.rs"
          "${CMAKE_SOURCE_DIR}/src/ast.rs"
          "${CMAKE_SOURCE_DIR}/src/bytecode.rs"
          "${CMAKE_SOURCE_DIR}/src/ir.rs"
          "${CMAKE_SOURCE_DIR}/src/lexer.rs"
          "${CMAKE_SOURCE_DIR}/src/main.rs"
          "${CMAKE_SOURCE_DIR}/src/os/linux/flavor.rs"
          "${CMAKE_SOURCE_DIR}/src/os/linux/link.rs"
          "${CMAKE_SOURCE_DIR}/src/os/linux/mod.rs"
          "${CMAKE_SOURCE_DIR}/src/os/linux/object.rs"
          "${CMAKE_SOURCE_DIR}/src/os/macos/link.rs"
          "${CMAKE_SOURCE_DIR}/src/os/macos/mod.rs"
          "${CMAKE_SOURCE_DIR}/src/os/macos/object.rs"
          "${CMAKE_SOURCE_DIR}/src/os/mod.rs"
          "${CMAKE_SOURCE_DIR}/src/resolver.rs"
          "${CMAKE_SOURCE_DIR}/src/rules.rs"
          "${CMAKE_SOURCE_DIR}/src/target.rs"
          "${CMAKE_SOURCE_DIR}/src/target/linux_aarch64/code.rs"
          "${CMAKE_SOURCE_DIR}/src/target/linux_aarch64/mod.rs"
          "${CMAKE_SOURCE_DIR}/src/target/linux_aarch64/plan.rs"
          "${CMAKE_SOURCE_DIR}/src/target/macos_aarch64/code.rs"
          "${CMAKE_SOURCE_DIR}/src/target/macos_aarch64/mod.rs"
          "${CMAKE_SOURCE_DIR}/src/target/macos_aarch64/plan.rs"
          "${CMAKE_SOURCE_DIR}/src/target/package_mfp/mod.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_collection_layout.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_collection_queries.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_collection_updates.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_control.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_conversions.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_fs_paths.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_math.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_misc.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_numeric.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_search.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_strings.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_strings_package.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/builder_values.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/code/mod.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/lower.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/mod.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/nir.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/plan.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/runtime.rs"
          "${CMAKE_SOURCE_DIR}/src/target/shared/validate.rs"
          "${CMAKE_SOURCE_DIR}/src/typecheck.rs"
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
