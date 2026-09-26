use std::cmp::Ordering;
#[cfg(test)]
use std::collections::HashSet;
use std::fmt;

use merc_io::LargeFormatter;
use oxidd::BooleanFunction;
use oxidd::Edge;
use oxidd::Function;
use oxidd::HasLevel;
use oxidd::InnerNode;
use oxidd::LevelNo;
use oxidd::Manager;
use oxidd::ManagerRef;
use oxidd::Node;
use oxidd::VarNo;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use oxidd::ldd::LDDFunction;
use oxidd::ldd::LDDManagerRef;
use oxidd::ldd::Value;
use oxidd::util::Borrowed;
use oxidd::util::OutOfMemory;
use oxidd::util::SatCountCache as OxiddSatCountCache;
use oxidd_core::function::EdgeOfFunc;
use oxidd_core::util::EdgeDropGuard;
use oxidd_core::util::num::F64;
use oxidd_rules_ldd::LDDTerminal;
use rustc_hash::FxBuildHasher;
use rustc_hash::FxHashMap;

/// Result of [`approx_satcount`] and [`ldd_len`], either an exact integer count or an f64 approximation.
///
/// The underlying [`BooleanFunction::sat_count`] initializes its accumulator to
/// `2^vars`, so a u64 accumulator overflows once `vars >= 64` regardless of the
/// actual count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SatCount {
    Exact(u128),
    Approximate(f64),
}

impl SatCount {
    /// The count converted to f64, lossy for [`SatCount::Exact`] values above `2^53`.
    pub fn as_f64(&self) -> f64 {
        match self {
            SatCount::Exact(n) => *n as f64,
            SatCount::Approximate(x) => *x,
        }
    }

    /// The exact count, or `None` if only an f64 approximation is available.
    pub fn exact(&self) -> Option<u128> {
        match self {
            SatCount::Exact(n) => Some(*n),
            SatCount::Approximate(_) => None,
        }
    }
}

impl fmt::Display for SatCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SatCount::Exact(n) => write!(f, "{}", LargeFormatter(*n)),
            SatCount::Approximate(x) => write!(f, "~{:e}", x),
        }
    }
}

/// Reusable cache for [`approx_satcount`], holding both the exact (`u64`) and the
/// approximate (`f64`) sub-caches so the same instance can serve calls with
/// any number of variables.
#[derive(Default)]
pub struct SatCountCache {
    exact: OxiddSatCountCache<u64, FxBuildHasher>,
    approximate: OxiddSatCountCache<F64, FxBuildHasher>,
}

impl SatCountCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Counts the number of satisfying assignments of `bdd` over `vars` variables.
///
/// Returns an exact [`SatCount::Exact`] when `vars < 64`, and falls back to
/// [`SatCount::Approximate`] otherwise (see [`SatCount`] for the reason).
pub fn approx_satcount(bdd: &BDDFunction, vars: VarNo, cache: &mut SatCountCache) -> SatCount {
    if vars < 64 {
        SatCount::Exact(bdd.sat_count::<u64, FxBuildHasher>(vars, &mut cache.exact).into())
    } else {
        SatCount::Approximate(bdd.sat_count::<F64, FxBuildHasher>(vars, &mut cache.approximate).0)
    }
}

/// Reusable cache for [`ldd_len`], the counterpart of [`SatCountCache`] for LDDs. Its entries
/// belong to the LDD manager of the counted sets, so it must not be shared with another manager.
#[derive(Default)]
pub struct LddLenCache {
    exact: OxiddSatCountCache<u128, FxBuildHasher>,
    approximate: OxiddSatCountCache<F64, FxBuildHasher>,
}

impl LddLenCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Counts the number of vectors in `ldd`.
///
/// Returns [`SatCount::Exact`] as long as the count comfortably fits in a `u128`, and falls back
/// to [`SatCount::Approximate`] otherwise. Counting in `u128` directly would silently wrap in a
/// release build, so the count is first computed in `f64`, which cannot overflow, and the exact
/// count is only computed if the approximation shows that it fits.
pub fn ldd_len(ldd: &LDDFunction, cache: &mut LddLenCache) -> SatCount {
    // Powers of two up to `2^100` are exactly representable, and an approximation below it
    // leaves plenty of room for the rounding error of the `f64` accumulation.
    const EXACT_LIMIT: f64 = (1u128 << 100) as f64;

    let approximate = ldd.len::<F64, FxBuildHasher>(&mut cache.approximate).0;
    if approximate < EXACT_LIMIT {
        SatCount::Exact(ldd.len::<u128, FxBuildHasher>(&mut cache.exact))
    } else {
        SatCount::Approximate(approximate)
    }
}

/// Computes the support (set of variables) of the given BDD function.
///
/// # Details
///
/// The `support` is the variables on which the `BDD` is defined, and other
/// variables are irrelevant or don't care, formally:
///
/// > support(f) = { x_i | exists x_0, ..., x_{i-1}, x_{i+1}, ..., x_n : f(x_0, ..., x_{i-1}, true, x_{i+1}, ..., x_n) != f(x_0, ..., x_{i-1}, false, x_{i+1}, ..., x_n) }
#[cfg(test)]
pub(crate) fn support(manager_ref: &BDDManagerRef, function: &BDDFunction) -> Result<Vec<VarNo>, OutOfMemory> {
    let mut result = HashSet::new();
    manager_ref.with_manager_shared(|manager| {
        support_edge(manager, function.as_edge(manager).borrowed(), &mut result);
    });
    Ok(result.into_iter().collect())
}

/// Recursive implementation of [support].
#[cfg(test)]
fn support_edge<'id>(
    manager: &<BDDFunction as Function>::Manager<'id>,
    function: Borrowed<EdgeOfFunc<'id, BDDFunction>>,
    result: &mut HashSet<VarNo>,
) {
    match manager.get_node(&function) {
        Node::Terminal(_) => (),
        Node::Inner(node) => {
            result.insert(node.level());

            // Recurse into cofactors
            let (high, low) = collect_children(node);
            support_edge(manager, low, result);
            support_edge(manager, high, result);
        }
    }
}

pub type Substitution = [(VarNo, VarNo)];

/// Specialized substitution function for variables renaming that only works
/// when the target variable is on the level directly below the renamed
/// variable.
///
/// # Details
///
/// In general substitution is defined as follows,
///
/// > f[x <- g] = (!g ∧ f[x <- false]) ∨ (g ∧ f[x <- true])
///
/// but its computation can be fairly expensive. Restricting the substitution to
/// only renaming variables from 'x' to the variable directly below allows for a
/// more efficient implementation, as follows:
///
/// > `f[x <- x+1] = (!x+1 ∧ f[x <- false]) ∨ (x+1 ∧ f[x <- true])`
/// > `            = make_node(x+1, f[x <- true][x+1 <- true], f[x <- false][x+1 <- false])`
///
/// where `x+1` denotes the level below `x`. The variable pairs are translated
/// to level pairs internally, so the renaming remains correct after the
/// variable order has been changed, as long as each target variable is directly
/// below the renamed variable in the current order.
pub fn variable_rename(
    manager_ref: &BDDManagerRef,
    function: &BDDFunction,
    substitution: &Substitution,
) -> Result<BDDFunction, OutOfMemory> {
    manager_ref.with_manager_shared(|manager| -> Result<BDDFunction, OutOfMemory> {
        // The recursion works on levels; variable numbers only coincide with
        // levels while no reordering has taken place.
        let mut levels: Vec<(LevelNo, LevelNo)> = substitution
            .iter()
            .map(|(from, to)| (manager.var_to_level(*from), manager.var_to_level(*to)))
            .collect();
        levels.sort_unstable();

        // Every substitution must be to the level directly below.
        for (from, to) in &levels {
            debug_assert!(from + 1 == *to, "Variable renaming must be to the level directly below");
        }

        let mut cache = FxHashMap::default();

        Ok(BDDFunction::from_edge(
            manager,
            variable_rename_edge(manager, &mut cache, function.as_edge(manager).borrowed(), &levels)?,
        ))
    })
}

/// Implementation of [variable_rename]. The `substitution` contains pairs of
/// *levels* (not variable numbers), sorted ascendingly.
///
/// # Cache key
///
/// The cache is keyed only on the BDD node (not on the substitution slice)
/// because `from < to`: as we traverse top-down, any node at level `L` has
/// already consumed all entries with `from < L`, either at the matching
/// `from`-level node or via the `level > from` branch.  The remaining
/// substitution suffix is therefore uniquely determined by the node's level
/// alone, so two paths to the same node always arrive with the same slice.
pub fn variable_rename_edge<'id>(
    manager: &<BDDFunction as Function>::Manager<'id>,
    cache: &mut FxHashMap<BDDFunction, BDDFunction>,
    function: Borrowed<EdgeOfFunc<'id, BDDFunction>>,
    substitution: &Substitution,
) -> Result<EdgeOfFunc<'id, BDDFunction>, OutOfMemory> {
    let node = match manager.get_node(&function) {
        Node::Terminal(terminal) => return manager.get_terminal(terminal),
        Node::Inner(node) => node,
    };

    if let Some(cached) = cache.get(&BDDFunction::from_edge(manager, manager.clone_edge(&function))) {
        return Ok(manager.clone_edge(cached.as_edge(manager)));
    }

    let (from, to) = match substitution.first() {
        None => {
            // No variables to substitute, so remains identity.
            return Ok(manager.clone_edge(&function));
        }
        Some((from, to)) => (from, to),
    };

    let result = if node.level() == *from {
        let (high, low) = collect_children(node);
        // Rename variable from 'from' to 'to'.
        let high = EdgeDropGuard::new(manager, variable_rename_edge(manager, cache, high, &substitution[1..])?);
        let low = EdgeDropGuard::new(manager, variable_rename_edge(manager, cache, low, &substitution[1..])?);

        let high_high = match manager.get_node(&high) {
            Node::Inner(node) => {
                if node.level() == *to {
                    // This is f[x <- true][x+1 <- true]
                    collect_children(node).0
                } else {
                    high.borrowed()
                }
            }
            Node::Terminal(_terminal) => high.borrowed(),
        };

        let low_low = match manager.get_node(&low) {
            Node::Inner(node) => {
                if node.level() == *to {
                    // There are f[x <- false][x+1 <- false]
                    collect_children(node).1
                } else {
                    low.borrowed()
                }
            }
            Node::Terminal(_terminal) => low.borrowed(),
        };

        reduce(
            manager,
            *to,
            manager.clone_edge(&high_high),
            manager.clone_edge(&low_low),
        )
    } else if node.level() > *from {
        // We are past the substitution point, so just continue.
        variable_rename_edge(manager, cache, function.borrowed(), &substitution[1..])
    } else {
        // node.level() < *from, in this case we keep the variable as is.
        let (high, low) = collect_children(node);
        let high = variable_rename_edge(manager, cache, high, substitution)?;
        let low = variable_rename_edge(manager, cache, low, substitution)?;

        reduce(manager, node.level(), high, low)
    }?;

    cache.insert(
        BDDFunction::from_edge(manager, manager.clone_edge(&function)),
        BDDFunction::from_edge(manager, manager.clone_edge(&result)),
    );

    Ok(result)
}

/// Specialized substitution function for variables renaming that only works
/// when the target variable is on the level directly above the renamed
/// variable, similar to [variable_rename].
///
/// # Details
///
/// We can derive the following:
///
/// > `f[x+1 <- x] = (!x ∧ f[x+1 <- false]) ∨ (x ∧ f[x+1 <- true])`
/// > `            = make_node(x, f[x+1 <- true][x <- true] , f[x+1 <- false][x <- false])`
///
/// where `x+1` denotes the level below `x`. The variable pairs are translated
/// to level pairs internally, as in [variable_rename].
pub fn variable_rename_reverse(
    manager_ref: &BDDManagerRef,
    function: &BDDFunction,
    substitution: &Substitution,
) -> Result<BDDFunction, OutOfMemory> {
    manager_ref.with_manager_shared(|manager| -> Result<BDDFunction, OutOfMemory> {
        // The recursion works on levels; variable numbers only coincide with
        // levels while no reordering has taken place.
        let mut levels: Vec<(LevelNo, LevelNo)> = substitution
            .iter()
            .map(|(from, to)| (manager.var_to_level(*from), manager.var_to_level(*to)))
            .collect();
        levels.sort_unstable();

        // Every substitution must be to the level directly above.
        for (from, to) in &levels {
            debug_assert!(*from == to + 1, "Variable renaming must be to the level directly above");
        }

        let mut cache = FxHashMap::default();

        Ok(BDDFunction::from_edge(
            manager,
            variable_rename_reverse_edge(manager, &mut cache, function.as_edge(manager).borrowed(), &levels)?,
        ))
    })
}

/// Implementation of [variable_rename_reverse]. The `substitution` contains
/// pairs of *levels* (not variable numbers), sorted ascendingly.
///
/// # Cache key
///
/// The cache is keyed on `(BDDFunction, &Substitution)` — the node *and* the
/// current substitution slice — because `to < from`: the `to`-level appears
/// above the `from`-level in the BDD.  A node at level `> from` can be reached
/// via two distinct paths:
///
/// 1. Through a `to`-level node, which consumes the entry and passes
///    `substitution[1..]` to its cofactors.
/// 2. Directly from an ancestor above `to` that skips the `to`-level (because
///    the function doesn't depend on that variable along that path), which
///    preserves the full slice.
///
/// The same BDD node can therefore be visited with different substitution
/// slices, so the slice must be part of the cache key to avoid returning a
/// stale result.
pub fn variable_rename_reverse_edge<'id, 'a>(
    manager: &<BDDFunction as Function>::Manager<'id>,
    cache: &mut FxHashMap<(BDDFunction, &'a Substitution), BDDFunction>,
    function: Borrowed<EdgeOfFunc<'id, BDDFunction>>,
    substitution: &'a Substitution,
) -> Result<EdgeOfFunc<'id, BDDFunction>, OutOfMemory> {
    let node = match manager.get_node(&function) {
        Node::Terminal(terminal) => return manager.get_terminal(terminal),
        Node::Inner(node) => node,
    };

    if let Some(cached) = cache.get(&(
        BDDFunction::from_edge(manager, manager.clone_edge(&function)),
        substitution,
    )) {
        return Ok(manager.clone_edge(cached.as_edge(manager)));
    }

    let (from, to) = match substitution.first() {
        None => {
            // No variables to substitute, identity.
            return Ok(manager.clone_edge(&function));
        }
        Some((from, to)) => (from, to),
    };

    let result = if node.level() == *to {
        // Build node at level `to` using cofactors of `from` where present.
        let (high, low) = collect_children(node);

        let high = EdgeDropGuard::new(
            manager,
            variable_rename_reverse_edge(manager, cache, high, &substitution[1..])?,
        );
        let low = EdgeDropGuard::new(
            manager,
            variable_rename_reverse_edge(manager, cache, low, &substitution[1..])?,
        );

        let high_high = match manager.get_node(&high) {
            Node::Inner(node) if node.level() == *from => collect_children(node).0,
            _ => high.borrowed(),
        };
        let low_low = match manager.get_node(&low) {
            Node::Inner(node) if node.level() == *from => collect_children(node).1,
            _ => low.borrowed(),
        };

        reduce(
            manager,
            *to,
            manager.clone_edge(&high_high),
            manager.clone_edge(&low_low),
        )
    } else if node.level() == *from {
        // `x+1` appears: rename this node to level `to`.
        let (high, low) = collect_children(node);
        let high = variable_rename_reverse_edge(manager, cache, high, &substitution[1..])?;
        let low = variable_rename_reverse_edge(manager, cache, low, &substitution[1..])?;
        reduce(manager, *to, high, low)
    } else if node.level() > *from {
        // Past both `to` and `from`, drop this substitution.
        variable_rename_reverse_edge(manager, cache, function.borrowed(), &substitution[1..])
    } else {
        // Recurse normally, keeping the substitution.
        let (high, low) = collect_children(node);
        let high = variable_rename_reverse_edge(manager, cache, high, substitution)?;
        let low = variable_rename_reverse_edge(manager, cache, low, substitution)?;
        reduce(manager, node.level(), high, low)
    }?;

    cache.insert(
        (
            BDDFunction::from_edge(manager, manager.clone_edge(&function)),
            substitution,
        ),
        BDDFunction::from_edge(manager, manager.clone_edge(&result)),
    );

    Ok(result)
}

/// Collect the two children (high, low) of a binary node
#[inline]
#[must_use]
pub(crate) fn collect_children<E: Edge, N: InnerNode<E>>(node: &N) -> (Borrowed<'_, E>, Borrowed<'_, E>) {
    debug_assert_eq!(N::ARITY, 2);
    let mut it = node.children();
    let f_then = it.next().unwrap();
    let f_else = it.next().unwrap();
    debug_assert!(it.next().is_none());
    (f_then, f_else)
}

/// Apply the reduction rules, creating a node in `manager` if necessary
#[inline(always)]
pub(crate) fn reduce<'id>(
    manager: &<BDDFunction as Function>::Manager<'id>,
    level: LevelNo,
    t: EdgeOfFunc<'id, BDDFunction>,
    e: EdgeOfFunc<'id, BDDFunction>,
) -> Result<EdgeOfFunc<'id, BDDFunction>, OutOfMemory> {
    // We do not use `DiagramRules::reduce()` here, as the iterator is
    // apparently not fully optimized away.
    if t == e {
        manager.drop_edge(e);
        return Ok(t);
    }
    oxidd_core::LevelView::get_or_insert(
        &mut manager.level(level),
        <<BDDFunction as Function>::Manager<'id> as Manager>::InnerNode::new(level, [t, e], ()),
    )
}

/// Returns the height of the LDD tree.
pub fn height(manager: &LDDManagerRef, ldd: &LDDFunction) -> usize {
    manager.with_manager_shared(|manager| height_edge(manager, ldd.as_edge(manager).borrowed()))
}

/// The edge variant of [height].
pub fn height_edge<'id>(
    manager: &<LDDFunction as Function>::Manager<'id>,
    ldd: Borrowed<EdgeOfFunc<'id, LDDFunction>>,
) -> usize {
    match manager.get_node(&ldd) {
        Node::Terminal(_) => 0,
        Node::Inner(node) => {
            // All right siblings share the same height, so only the down chain
            // contributes to the height of the LDD.
            let (down, _right) = collect_children(node);
            1 + height_edge(manager, down)
        }
    }
}

/// Returns true iff the set contains the vector.
pub fn element_of(manager: &LDDManagerRef, vector: &[Value], ldd: &LDDFunction) -> bool {
    manager.with_manager_shared(|manager| element_of_edge(manager, vector, ldd.as_edge(manager).borrowed()))
}

fn element_of_edge<'id>(
    manager: &<LDDFunction as Function>::Manager<'id>,
    vector: &[Value],
    ldd: Borrowed<EdgeOfFunc<'id, LDDFunction>>,
) -> bool {
    match manager.get_node(&ldd) {
        Node::Terminal(LDDTerminal::True) => vector.is_empty(),
        Node::Terminal(LDDTerminal::Empty) => false,
        Node::Inner(node) => {
            let value = *node.get_value();
            let (down, right) = collect_children(node);
            match vector.first() {
                None => false,
                Some(&first) => match value.cmp(&first) {
                    Ordering::Less => element_of_edge(manager, vector, right),
                    Ordering::Equal => element_of_edge(manager, &vector[1..], down),
                    Ordering::Greater => false,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use merc_utilities::random_test;
    use oxidd::BooleanFunction;
    use oxidd::FunctionSubst;
    use oxidd::Manager;
    use oxidd::ManagerRef;
    use oxidd::Subst;
    use oxidd::VarNo;
    use oxidd::bdd::BDDFunction;
    use oxidd::util::AllocResult;
    use rand::RngExt;

    use crate::BDD_CACHE_CAPACITY;
    use crate::BDD_NODE_CAPACITY;
    use crate::FormatConfigSet;
    use crate::compute_vars_bdd;
    use crate::random_bdd;
    use crate::support;
    use crate::variable_rename;
    use crate::variable_rename_reverse;

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_bdd_variable_rename() {
        let manager_ref = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);

        let vars: Vec<BDDFunction> = manager_ref
            .with_manager_exclusive(|manager| {
                AllocResult::from_iter(manager.add_vars(3).map(|i| BDDFunction::var(manager, i)))
            })
            .unwrap();

        let res = vars[0].and(&vars[1]).unwrap().or(&vars[2]).unwrap();
        let subst = variable_rename(&manager_ref, &res, &[(0, 1), (1, 2)]).unwrap();
        assert!(subst.satisfiable());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_bdd_support() {
        random_test(25, |rng| {
            let manager_ref = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);

            let vars = manager_ref
                .with_manager_exclusive(|manager| {
                    manager
                        .add_vars(4)
                        .map(|v| BDDFunction::var(manager, v))
                        .collect::<Result<Vec<BDDFunction>, _>>()
                })
                .unwrap();

            let function = random_bdd(&manager_ref, rng, &vars, 8).unwrap();
            let _support = support(&manager_ref, &function).unwrap();

            // TODO: Verify support correctness
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_bdd_renaming() {
        random_test(25, |rng| {
            let manager_ref = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);

            let vars = manager_ref
                .with_manager_exclusive(|manager| {
                    manager
                        .add_vars(4)
                        .map(|v| BDDFunction::var(manager, v))
                        .collect::<Result<Vec<BDDFunction>, _>>()
                })
                .unwrap();

            let function = random_bdd(&manager_ref, rng, &vars, 8).unwrap();

            let to = compute_vars_bdd(&manager_ref, &[1, 3]).unwrap().0;
            let substitution = Subst::new(&[0, 2], &to);
            println!("input: {}", FormatConfigSet(&function));

            let expected = function.substitute(&substitution).unwrap();
            let renamed = variable_rename(&manager_ref, &function, &[(0, 1), (2, 3)]).unwrap();

            println!("expected: {}", FormatConfigSet(&expected));
            println!("result: {}", FormatConfigSet(&renamed));

            assert!(expected == renamed, "Renaming did not match expected substitution");
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_bdd_renaming_reverse() {
        random_test(25, |rng| {
            let manager_ref = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);

            let vars = manager_ref
                .with_manager_exclusive(|manager| {
                    manager
                        .add_vars(4)
                        .map(|v| BDDFunction::var(manager, v))
                        .collect::<Result<Vec<BDDFunction>, _>>()
                })
                .unwrap();

            let function = random_bdd(&manager_ref, rng, &vars, 8).unwrap();

            let to = compute_vars_bdd(&manager_ref, &[0, 2]).unwrap().0;
            let substitution = Subst::new(&[1, 3], &to);
            println!("input: {}", FormatConfigSet(&function));

            let expected = function.substitute(&substitution).unwrap();
            let renamed = variable_rename_reverse(&manager_ref, &function, &[(1, 0), (3, 2)]).unwrap();

            println!("expected: {}", FormatConfigSet(&expected));
            println!("renamed: {}", FormatConfigSet(&renamed));

            assert!(
                expected == renamed,
                "Renaming with reverse did not match expected substitution"
            );
        });
    }

    /// [test_random_bdd_renaming] and [test_random_bdd_renaming_reverse] only ever exercise the
    /// fixed substitution `[(0, 1), (2, 3)]` (two entries, no gap between them): every random
    /// iteration varies the BDD, never the substitution shape. [`variable_rename_edge`]'s cache key
    /// is only sound (see its doc comment) because two different paths to the same node always
    /// arrive with the same remaining substitution *suffix*; this varies the number of entries and
    /// the gaps between them (both affect which suffix a node sees) to stress that invariant.
    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_bdd_renaming_varied_substitutions() {
        random_test(100, |rng| {
            let manager_ref = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);

            let num_vars = 10;
            let vars = manager_ref
                .with_manager_exclusive(|manager| {
                    manager
                        .add_vars(num_vars)
                        .map(|v| BDDFunction::var(manager, v))
                        .collect::<Result<Vec<BDDFunction>, _>>()
                })
                .unwrap();

            let function = random_bdd(&manager_ref, rng, &vars, 16).unwrap();

            // Five disjoint (from, from+1) blocks spanning all 10 variables; a random non-empty
            // subset, at random gaps from each other, is a valid substitution regardless of which
            // blocks are picked.
            let blocks: Vec<(VarNo, VarNo)> = (0..5).map(|k| (2 * k, 2 * k + 1)).collect();
            let mut substitution: Vec<(VarNo, VarNo)> =
                blocks.iter().copied().filter(|_| rng.random_bool(0.5)).collect();
            if substitution.is_empty() {
                substitution.push(blocks[rng.random_range(0..blocks.len())]);
            }

            let from: Vec<VarNo> = substitution.iter().map(|(f, _)| *f).collect();
            let to_targets: Vec<VarNo> = substitution.iter().map(|(_, t)| *t).collect();
            let to = compute_vars_bdd(&manager_ref, &to_targets).unwrap().0;
            let oracle_substitution = Subst::new(&from, &to);

            let expected = function.substitute(&oracle_substitution).unwrap();
            let renamed = variable_rename(&manager_ref, &function, &substitution).unwrap();

            assert!(
                expected == renamed,
                "substitution {substitution:?}: renaming did not match expected substitution"
            );

            // Mirror the same varied shapes through the reverse direction: (from, to) becomes
            // (from+1, from), i.e. renaming the variable directly below back up to `from`.
            let reverse_substitution: Vec<(VarNo, VarNo)> = substitution.iter().map(|&(f, t)| (t, f)).collect();
            let reverse_from: Vec<VarNo> = reverse_substitution.iter().map(|(f, _)| *f).collect();
            let reverse_to_targets: Vec<VarNo> = reverse_substitution.iter().map(|(_, t)| *t).collect();
            let reverse_to = compute_vars_bdd(&manager_ref, &reverse_to_targets).unwrap().0;
            let reverse_oracle = Subst::new(&reverse_from, &reverse_to);

            let reverse_expected = function.substitute(&reverse_oracle).unwrap();
            let reverse_renamed = variable_rename_reverse(&manager_ref, &function, &reverse_substitution).unwrap();

            assert!(
                reverse_expected == reverse_renamed,
                "reverse substitution {reverse_substitution:?}: renaming did not match expected substitution"
            );
        });
    }
}
