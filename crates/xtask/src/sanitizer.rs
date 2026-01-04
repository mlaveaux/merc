use std::error::Error;
use std::path::Path;

pub use duct::cmd;

#[allow(clippy::ptr_arg)]
fn add_target_flag(_arguments: &mut Vec<String>) {
    #[cfg(target_os = "linux")]
    {
        _arguments.push("--target".to_string());
        _arguments.push("x86_64-unknown-linux-gnu".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        _arguments.push("--target".to_string());
        _arguments.push("x86_64-apple-darwin".to_string());
    }
}

///
/// Run the tests with the address sanitizer enabled to detect memory issues in unsafe code.
///
/// This only works under Linux and MacOS currently and requires the nightly toolchain.
///
pub fn address_sanitizer(mut arguments: Vec<String>) -> Result<(), Box<dyn Error>> {
    arguments.extend(vec!["-Zbuild-std".to_string()]);

    add_target_flag(&mut arguments);

    let leak_sanitizer_suppress = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/leak_sanitizer.suppress");

    cmd("cargo", arguments)
        .env(
            "LSAN_OPTIONS",
            format!("suppressions={}", leak_sanitizer_suppress.to_string_lossy()),
        )
        .env("RUSTFLAGS", "-Zsanitizer=address,leak")
        .env("RUSTDOCFLAGS", "-Zsanitizer=address,leak")
        .env("CFLAGS", "-fsanitize=address,leak")
        .env("CXXFLAGS", "-fsanitize=address,leak")
        .run()?;
    println!("ok.");

    Ok(())
}

///
/// Run the tests with the thread sanitizer enabled to detect data race conditions.
///
/// This only works under Linux and MacOS currently and requires the nightly toolchain.
///
pub fn thread_sanitizer(mut arguments: Vec<String>) -> Result<(), Box<dyn Error>> {
    arguments.extend(vec!["-Zbuild-std".to_string()]);

    add_target_flag(&mut arguments);

    let thread_sanitizer_suppress = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/thread_sanitizer.suppress");

    cmd("cargo", arguments)
        .env(
            "TSAN_OPTIONS",
            format!("suppressions={}", thread_sanitizer_suppress.to_string_lossy()),
        )
        .env("RUSTFLAGS", "-Zsanitizer=thread")
        .env("RUSTDOCFLAGS", "-Zsanitizer=thread")
        .env("CFLAGS", "-fsanitize=thread")
        .env("CXXFLAGS", "-fsanitize=thread")
        .run()?;
    println!("ok.");

    Ok(())
}
