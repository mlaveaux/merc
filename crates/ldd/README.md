 # Overview

A library to create and manipulate so called list decision diagrams,
abbreviated as LDDs. List decision diagrams are data structures that can
efficiently represent sets of vectors containing natural numbers \[Dijk18\].

An LDD is inductively defined as follows. First of all, constants 'true' and
'false' are two distinct LDDs. Given LDDs x and y, then node(value, x, y) is
an LDD; where value is a natural number. As such, we observe that node(5,
true, node(4, true, false)) is an LDD and in general we obtain a tree-like
data structure.

Next, we should explain how LDDs represent a set of vectors. Given an LDD n
then \[n\] is inductively defined as:

```plain
 [false]                    = ∅
 [true]                     = { <> }
 [node(value, down, right)] = { value x | x in [down] } ∪ [right]\
```

Node that since 'true' and 'false' are not very insightful and clash with
Rust keywords we use 'empty vector' and 'empty set' for the constants 'true'
and 'false' respectively.

## Citations

> \[Dijk18\] --- "Sylvan: multi-core framework for decision diagrams". Tom van Dijk, Jaco van de Pol. International Journal on Software Tools for Technology Transfer. 19(6):675-696, 2017.

## Safety

This crate contains no `unsafe` code.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may be upgraded at any time when necessary.

## License

All MERC crates are licensed under the `BSL-1.0` license. See the [LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE) file in the repository root for more information.