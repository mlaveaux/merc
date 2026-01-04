# Overview

This crate contains functionality for manipulating labelled transition systems,
including writing and reading LTSs from files. A labelled transition system is a
tuple:

> (s0, S, Act, T), where `s0` is the initial state, `S` is a set of states, `Act` is a set of action labels, and T is a set of transitions T ⊆ S × Act × S.

The main concept of this crate is the central `LTS` trait that encapsulates
labelled transition systems with generic action label types. We use `strong`
types for the various indices (states, actions, etc) to avoid mixing them up at
compile time. This is implemented using the `TagIndex` type of the
`merc_utilities` crate. This crate also deals with the special `τ` (or `tau`)
action that is used to model internal actions.

The crate supports reading and writing LTSs in both the mCRL2 binary
[`.lts`](https://www.mcrl2.org/web/user_manual/tools/lts.html) format and the
**AUT**omaton [`.aut`](https://cadp.inria.fr/man/aut.html) (also called ALDEBARAN)
format. For the mCRL2 format the action label is a `MultiAction` to account for
multi-actions. Furthermore, the crate also contains an `LtsBuilder` that can be
used to generate LTSs programmatically. The crate also uses
compressed vectors internally to store transitions memory efficiently.

An example for using the `LtsBuilder` to create a simple LTS is shown below:

```rust
use merc_lts::LTS;
use merc_lts::LtsBuilder;
use merc_lts::StateIndex;

let mut builder = LtsBuilder::new(vec!["a".to_string(), "b".to_string()], vec![]);
builder.add_transition(StateIndex::new(0), "a", StateIndex::new(1));
builder.add_transition(StateIndex::new(1), "b", StateIndex::new(1));

let lts = builder.finish(StateIndex::new(0));
assert_eq!(lts.num_of_states(), 2);
assert_eq!(lts.num_of_transitions(), 2);
```

## Features

The `cadp` feature flag enables support for reading and writing LTSs in the
BCG format from the [CADP](https://cadp.inria.fr/man/bcg.html) toolset. The `CADP` environment
variable must be set to the installation path of CADP at compilation and run time for this to work.

## Changelog

### Current

Added support for reading and writing LTSs in the BCG format from the
[CADP](https://cadp.inria.fr/man/bcg.html) toolset. This requires CADP to be
installed on the system and the `CADP` environment variable to be set, and it is
enabled via the `cadp` feature flag.

Added the `clap` feature to conditionally enable the `clap` dependency to derive
some convenience traits.

Made the AUT format compliant with the actual specification. In particular,
the internal action is now represented by `i` instead of `tau`.

Introduce a proper `MultiAction` type to represent multi-actions as they are
present in mCRL2. A multi-action is a multi-set of action labels that are
executed simultaneously, i.e., an action `a|b` is a multi-action consisting of
the actions `a` and `b`. Note that `a|b` is equivalent to `b|a` in this
formalism, and `τ` simply denotes the empty set.

### v1.1.0

Removed `add_transition_index` from `LtsBuilder` and `LtsBuilderFast` since it
is not correct. These builders change the indices of labels internally to
accommodate for the internal actions.

## Safety

This crate contains minimal `unsafe` code for FFI with CADP, but every other module
forbids `unsafe` code.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may be upgraded at any time when necessary.

## License

All MERC crates are licensed under the `BSL-1.0` license. See the [LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE) file in the repository root for more information.