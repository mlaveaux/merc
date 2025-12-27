# Overview

Internal crate for the MERC toolset the provides utility types and functions for
the Merc toolset.

One important utility is the `MercError` type, which is a common error type used
throughout the MERC toolset. This type provides thin pointers for `dyn Error`
trait objects, which helps to reduce memory usage and improve performance when
handling errors. Furthermore, it provides a stack trace by default, which can be
very useful for debugging and diagnosing issues.

Another important testing function is the `random_test` function, which can be
used in tests to provide (reproducable) random state. This is useful for testing
code that relies on randomness, as it allows for consistent and repeatable
tests.

# Safety

This crate contains no unsafe code. If unsafe code is needed it should be in the
`merc_unsafety` crate.