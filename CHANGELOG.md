# Current

Added support for reading and writing LTSs in the `.bcg` format used by the [CADP](https://cadp.inria.fr/) toolset, this is gated behind the `cadp` feature flag.

Made the AUT format compliant with the original specification. In particular, the internal action is now represented by `i` instead of `tau`. This means that `merc-lts` now requires the `--tau=tau` flag to read AUT files that use `tau` as the internal action, as is the case for mCRL2.

This release introduces the `merc_preorder` crate, which can be used to check whether two labelled transition systems are related by various pre-order relations, which are (weak) trace, failure refinement and failures-divergence refinement and impossible futures.

See the `README.md` of the individual crates for their own changelogs.

# v1.0 (December 2025)

The initial release of the MERC toolset. Although no where near feature complete we opted to produce a 1.0 release to mark the initial milestone of the toolset instead of staying in zero version forever. We generally expect to release a single major version per year, without focusing too much on avoiding breaking changes to libraries for foreseeable future.

This release comes with a set of five tools:
  - `merc-lts` implement branching, strong and weak (signature-based) bisimulation reduction and comparison for labelled transition systems in the mCRL2 binary [`.lts`](https://www.mcrl2.org/web/user_manual/tools/lts.html) format and the **AUT**omaton (or ALDEBARAN) [`.aut`](https://cadp.inria.fr/man/aut.html) format.
  - `merc-rewrite` allows rewriting of Rewrite Engine Competition specifications ([REC](https://doi.org/10.1007/978-3-030-17502-3_6)) using [Sabre](https://arxiv.org/abs/2202.08687) (Set Automaton Based Rewrite Engine).
  - `merc-vpg` can be used to solve (variability) parity games in the [PGSolver](https://github.com/tcsprojects/pgsolver) `.pg` format, and a slightly extended variability parity game `.vpg` format. Furthermore, it can generate variability parity games for model checking modal mu-calculus on LTSs.
  - `merc-pbes` can identify symmetries in parameterised boolean equation systems [PBES](https://doi.org/10.1016%2Fj.tcs.2005.06.016).
  - `merc-ltsgraph` is a GUI tool to visualize LTSs.

This release also comes with various crates that can be used, these are also published on [crates.io](https://crates.io/users/mlaveaux):
  - `merc_aterm`: A feature complete thread-safe re-implementation of the mCRL2 aterm library in Rust. Can read and write aterms in (streamable) binary and text formats.
  - `merc_data`: Defines data expressions mimicking the mCRL2 data expressions, demonstrates the use of Rust macros (defined in `merc_macros`) to generate the required boiler plate code.    
  - `merc_lts`: library for manipulating labelled transition systems, including I/O for mCRL2's `.lts` files and Aldebaran `.aut` files. Can also generated random LTSs and (synchronous) products for testing purposes.
  - `merc_sharedmutex` is a crate implementing the busy-forbidden protocol, a read-efficient readers-writer lock described in the [preprint](https://arxiv.org/pdf/2111.02706).
  - `merc_syntax` defines a [Pest](https://pest.rs/) grammar for the mCRL2 specification, modal formula and PBES languages, and defines an AST for these languages.
  - `merc_vpg` defines functionality for manipulating (variability) parity games, including I/O for the PGSolver `.pg` format and a slightly extended variability parity game `.vpg` format.

**Full Changelog**: https://github.com/MERCorg/merc/commits/v1.0