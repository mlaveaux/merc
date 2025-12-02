cmake_minimum_required(VERSION 3.14)

project(merc_ffi
    VERSION 1.0.0
    DESCRIPTION "Foreign function interface for merc"
    LANGUAGES CXX)
    
# Mark some cmake variables as advanced since they don't have to be exposed to the user.
mark_as_advanced(FORCE
    CORROSION_BUILD_TESTS
    CORROSION_DEV_MODE
    CORROSION_NATIVE_TOOLING
    CORROSION_NO_WARN_PARSE_TARGET_TRIPLE_FAILED
    CORROSION_RESPECT_OUTPUT_DIRECTORY
    CORROSION_VERBOSE_OUTPUT
    Rust_RESOLVE_RUSTUP_TOOLCHAINS)

# Use FetchContent to acquire Corrosion for the cmake integration of Rust projects.
include(FetchContent)

FetchContent_Declare(corrosion
    GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
    GIT_TAG v0.5.2)
FetchContent_MakeAvailable(corrosion)

find_program(CBINDGEN_EXECUTABLE
    NAMES cbindgen
    DOC "Requires cbindgen to generate the OxiDD headers"
    REQUIRED)

set(MERC_CARGO_PROFILE "release"
    CACHE STRING "Cargo profile to use for building the merc-ffi crate")
set_property(CACHE MERC_CARGO_PROFILE PROPERTY STRINGS "dev" "release")

set(MERC_INCLUDE_DIR ${PROJECT_BINARY_DIR}/include)
set(MERC_INCLUDE_HEADER ${MERC_INCLUDE_DIR}/merc_ffi.h)
set(MERC_ROOT_DIR "${PROJECT_SOURCE_DIR}/../")

add_custom_command(OUTPUT ${MERC_INCLUDE_HEADER}
    COMMAND ${CBINDGEN_EXECUTABLE}
    ARGS ${MERC_ROOT_DIR}/crates/aterm-ffi/ --output ${MERC_INCLUDE_HEADER}
    DEPENDS ${MERC_ROOT_DIR}/crates/aterm-ffi/cbindgen.toml
            ${MERC_ROOT_DIR}/crates/aterm-ffi/src/lib.rs
    COMMENT "Generating C header for the merc_ffi crate"
    VERBATIM)
add_custom_target(merc_ffi_header ALL DEPENDS ${MERC_INCLUDE_HEADER})
    
# Import the merc_aterm-ffi crate specifically
corrosion_import_crate(
    MANIFEST_PATH ${MERC_ROOT_DIR}/Cargo.toml
    PROFILE ${MERC_CARGO_PROFILE}
    CRATES merc_aterm-ffi
    CRATE_TYPES staticlib)

add_dependencies(merc_aterm_ffi merc_ffi_header)
target_include_directories(merc_aterm_ffi INTERFACE
    ${MERC_INCLUDE_DIR}
    ${PROJECT_SOURCE_DIR}/include)

add_library(merc_ffi ALIAS merc_aterm_ffi)

add_subdirectory(example)