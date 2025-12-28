# Overview

This crate contains functionality for manipulating labelled transition systems,
including writing and reading LTSs from files. A labelled transition system is a
tuple (s0, S, Act, T) where `s0` is the initial state, `S` is a set of states,
`Act` is a set of action labels, and T is a set of transitions T ⊆ S × Act × S.
The main concept of this crate is the central `LTS` trait that encapsulates
labelled transition systems with generic action label types. This trait uses
`strong` types for the various indices used (states, actions, etc) to avoid
mixing them up at compile time. This is implemented using the `TagIndex` type of
the `merc_utilities` crate.

The crate supports reading and writing LTSs in both the mCRL2 binary
[`.lts`](https://www.mcrl2.org/web/user_manual/tools/lts.html) format and the
**AUT**omaton [`.aut`](https://cadp.inria.fr/man/aut.html) (also called ALDEBARAN)
format. For the mCRL2 format the action label is a `MultiAction` to account for
multi-actions. Furthermore, the crate also contains an `LtsBuilder` that can be
used to generate LTSs programmatically. Internally, the crate also uses
compressed vectors to store transitions memory efficiently.

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

The central `LTS` trait allows one to implement various algorithms on LTSs in a
generic way.

## Safety

This crate contains no unsafe code.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may be upgraded at any time when necessary.

## License

All MERC crates are licensed under the `BSL-1.0` license. See the [LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE) file in the repository root for more information.