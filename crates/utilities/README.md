# Overview

 > ⚠️ **important** This is an internal crate and is not intended for public use.

Internal crate for the MERC toolset the provides utility types and functions for
the Merc toolset.

One important utility is the `MercError` type, which is a common error type used
throughout the MERC toolset. This type provides thin pointers for `dyn Error`
trait objects, which helps to reduce memory usage and improve performance when
handling errors. Furthermore, it provides a stack trace by default, which can be
very useful for debugging and diagnosing issues.

An important testing function is the `random_test` function, which can be used
in tests to provide (reproducable) random state. This is useful for testing code
that relies on randomness, as it allows for consistent and repeatable tests.
Finally, it provides a `Timing` struct that can be used to measure and record
the time taken by various operations in the MERC toolset.

## Safety

This crate contains no unsafe code. If unsafe code is needed it should be in the
`merc_unsafety` crate.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may be upgraded at any time when necessary.

## License

All MERC crates are licensed under the `BSL-1.0` license. See the [LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE) file in the repository root for more information.