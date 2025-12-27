# Overview

Implements a readers-writer lock based on the busy-forbidden protocol, explained in the paper:

> A Thread-Safe Term Library: (with a New Fast Mutual Exclusion Protocol). Jan Friso Groote, Maurice Laveaux, Flip van Spaendonck. [arXiv](https://arxiv.org/abs/2111.02706).

A correctness proof of the protocol can be found in the follow-up paper:

> Verification of the busy-forbidden protocol (using an extension of the cones and foci framework). Flip van Spaendonck. [arXiv](https://doi.org/10.48550/ARXIV.2208.05334).

The implementation is extended with a read recursive lock variant called `RecursiveLock`, which allows a thread to acquire multiple read locks recursively without deadlocking. Furthermore, a `BfVec` has been introduced which allows efficient concurrent modifications to a vector using the busy-forbidden protocol.

Compared to other readers-writer locks this implementation requires local data for every reader, which is achieved by requiring the user to clone the readers-writer lock (once) for every thread that must have access to the shared state. This allows for very efficient (uncontended) read access, at the cost of more memory usage and typically higher overhead for write access. In its main use case of the term library, or large vectors, the amount of write accesses is often in the orders of 1000+ times less frequent than read accesses, making this trade-off worthwhile.

## Authors

This crate was developed by Maurice Laveaux, Flip van Spaendonck and Jan Friso Groote.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may be upgraded at any time when necessary.

## License

All MERC crates are licensed under the BSL-1.0 license. See the [LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE) file in the repository root for more information.