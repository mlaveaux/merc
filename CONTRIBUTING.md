# Contributing

Source code documentation can be found at Github
[pages](https://mlaveaux.github.io/merc/index.html), and more detailed
documentation can be found in `doc`. See `doc/TESTING.md` for more information
on how to run the tests.

## Formatting

All source code should be formatted using `cargo fmt`, which can installed using
`rustup component add rustfmt`. Source files can then be formatted using `cargo
+nightly fmt`, or a single crate with `-p <crate_name>`.

## Third party libraries

We generally strive for using high quality third party dependencies. For this
purpose we use `cargo deny check`, installed with `cargo install cargo-deny` to
check the license of third party libraries and to compare them to the `RustSec`
advisory db. In general unmaintained dependencies should either be vendored or
replaced by own code if possible. However, using third party libraries where
applicable is generally not discouraged.