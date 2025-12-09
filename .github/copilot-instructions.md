This is a Rust based repository that demonstrates efficient, correct and safe implementations of algorithms and data structures.

## Development Workflows

### Building & Testing

```bash
# Standard build
cargo build                    # Debug (dev) mode
cargo build --release          # Release mode

# GUI tools (separate workspace)
cd tools/gui && cargo build

# mCRL2 tools (separate workspace)
cd tools/mCRL2 && cargo build

# Testing
cargo test                     # All tests
cargo test -- --no-capture     # Show test output
cargo test -p merc_sabre --lib # Single crate
```

### Formatting & Quality

```bash
# Format code (required before commit)
cargo +nightly fmt
```

## Third-Party Dependencies

### Core Dependencies

- **`delegate`**: Trait delegation macros to reduce boilerplate
- **`rayon`**: Data parallelism and parallel iterators
- **`itertools`**: Extended iterator functionality
- **`thiserror`**: Ergonomic error type derivation

### Parsing & Grammar

- **`pest`/`pest_derive`**: PEG parser generator
- **`pest_consume`**: Vendored in `3rd-party/pest_consume/` - parser combinator framework built on pest
- Grammar files use `.pest` extension (see `crates/syntax/mcrl2_grammar.pest`, `crates/aterm/term_grammar.pest`)

### Data Structures

- **`hashbrown`**: Fast HashMap implementation (basis for std HashMap)
- **`dashmap`**: Concurrent HashMap
- **`smallvec`**: Stack-allocated vectors for small sizes
- **`bitvec`**: Bit manipulation and bit vectors
- **`oxidd`**: Binary decision diagrams (BDDs) for symbolic computation

### Memory & Allocation

- **`allocator-api2`**: Unstable allocator API support (used in `crates/unsafety/`)
- **`bumpalo`**: Bump allocation for arena-style memory

### CLI & I/O

- **`clap`** (with derive): Command-line argument parsing
- **`bitstream-io`**: Binary I/O for LTS file formats
- **`env_logger`/`log`**: Logging infrastructure

### Development & Testing

- **`criterion`**: Micro-benchmarking framework (see `crates/*/benchmarks/`)
- **`test-case`/`test-log`**: Parameterized tests and test logging
- **`arbtest`/`arbitrary`**: Property-based testing and fuzzing
- **`trybuild`**: Compile-fail tests for proc macros (see `crates/macros/tests/`)

### Build & Tasks

- **`duct`**: Shell command execution (used in xtask)
- **`proc-macro2`/`quote`/`syn`**: Proc macro development (see `crates/macros/`)
- **`regex`**: Regular expressions (benchmarking, parsing)
- **`serde`/`serde_json`**: Serialization (benchmark results, configs)

