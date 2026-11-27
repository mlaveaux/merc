use std::collections::HashMap;
use std::ops::ControlFlow;

use merc_syntax::ComplexSort;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::SortId;
use merc_syntax::Span;
use merc_syntax::Traverse;
use merc_syntax::UntypedDataSpecification;
use merc_utilities::Step;

/// An error found in the alias declarations by [check_aliases].
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum AliasError {
    /// The alias reaches itself through basic sorts, containers or function
    /// sorts, so expanding it does not terminate. The cycle starts at the
    /// offending alias and lists the aliases visited along the way.
    #[error("alias cycle through {cycle:?}")]
    Circular { cycle: Vec<SortId> },
    /// The alias reaches itself through a function sort, or a `Set` or `Bag`
    /// container, possibly via a structured sort. Such sorts have no sensible
    /// (cardinality-consistent) interpretation.
    #[error("sort {sort:?} is recursively defined via a function sort, or a set or a bag type container")]
    ThroughFunctionSort { sort: SortId },
}

/// Checks the alias declarations with two searches:
///
/// - Circularity: an alias may not reach itself through basic sorts,
///   containers or function sorts. Structured sorts terminate the search
///   because recursion through a constructor is well-defined, e.g.
///   `sort Tree = struct leaf | node(Tree, Tree);`.
/// - Function-sort loops: recursion through a function sort or a `Set`/`Bag`
///   container is rejected even when it passes through a structured sort, e.g.
///   `sort S = struct f(S -> Bool);`. A loop through a `List` (or `FSet`/`FBag`)
///   container is allowed.
///
/// Requires that all sort names in the specification have been resolved.
pub(crate) fn check_aliases(spec: &UntypedDataSpecification) -> Result<(), (AliasError, Span)> {
    let mut alias_map: HashMap<SortId, &SortExpression> = HashMap::new();
    for sort_decl in &spec.sort_declarations {
        if let Some(alias) = &sort_decl.expr {
            alias_map.insert(sort_decl.id.expect("Name must have been resolved"), alias);
        }
    }

    // Iterate in declaration order so the reported cycle is deterministic.
    for sort_decl in &spec.sort_declarations {
        if let Some(alias) = &sort_decl.expr {
            let lhs = sort_decl.id.expect("Name must have been resolved");
            let mut visited = Vec::new();
            check_function_sort_loop(lhs, alias, &mut visited, false, &alias_map)
                .map_err(|err| (err, sort_decl.span.clone()))?;
            debug_assert!(visited.is_empty());

            check_circularity(lhs, alias, &mut visited, &alias_map).map_err(|err| (err, sort_decl.span.clone()))?;
            debug_assert!(visited.is_empty());
        }
    }

    Ok(())
}

/// The circularity check: searches for `lhs` through aliases, containers and
/// function sorts, stopping at structured sorts.
fn check_circularity(
    lhs: SortId,
    rhs: &SortExpression,
    visited: &mut Vec<SortId>,
    alias_map: &HashMap<SortId, &SortExpression>,
) -> Result<(), AliasError> {
    rhs.visit_with::<(), (), AliasError, _>((), |expr, ()| match &expr.node {
        SortExpressionKind::Resolved(_, id) => {
            if *id == lhs {
                let mut cycle = vec![lhs];
                cycle.extend(visited.iter().copied());
                return Err(AliasError::Circular { cycle });
            }
            if !visited.contains(id)
                && let Some(alias) = alias_map.get(id)
            {
                visited.push(*id);
                check_circularity(lhs, alias, visited, alias_map)?;
                visited.pop();
            }
            Ok(ControlFlow::Continue(Step::Into(())))
        }
        // Recursion through a structured sort is well-defined, so the search
        // deliberately stops here.
        SortExpressionKind::Struct { .. } => Ok(ControlFlow::Continue(Step::Prune)),
        SortExpressionKind::Reference(_) => unreachable!("Names must have been resolved"),
        _ => Ok(ControlFlow::Continue(Step::Into(()))),
    })
    .map(|_| ())
}

/// The function-sort-loop check: searches for `lhs` through aliases,
/// containers, function sorts *and* structured sorts.
///
/// Reports a loop only when a function sort or a `Set`/`Bag` container was
/// passed along the way, indicated by the `is_function_like_sort` parameter.
fn check_function_sort_loop(
    lhs: SortId,
    rhs: &SortExpression,
    visited: &mut Vec<SortId>,
    is_function_like_sort: bool,
    alias_map: &HashMap<SortId, &SortExpression>,
) -> Result<(), AliasError> {
    rhs.visit_with::<bool, (), AliasError, _>(is_function_like_sort, |expr, observed| match &expr.node {
        SortExpressionKind::Resolved(_, id) => {
            if *id == lhs && observed {
                return Err(AliasError::ThroughFunctionSort { sort: lhs });
            }
            if !visited.contains(id)
                && let Some(alias) = alias_map.get(id)
            {
                visited.push(*id);
                check_function_sort_loop(lhs, alias, visited, observed, alias_map)?;
                visited.pop();
            }
            Ok(ControlFlow::Continue(Step::Into(observed)))
        }
        // The container kind *replaces* the flag, as in mCRL2: passing through
        // a List (or FSet/FBag) resets an earlier function-sort observation, so
        // `struct f(Bool -> List(S))` is accepted.
        SortExpressionKind::Complex(op, _) => Ok(ControlFlow::Continue(Step::Into(matches!(
            op,
            ComplexSort::Set | ComplexSort::Bag
        )))),
        SortExpressionKind::Function { .. } | SortExpressionKind::FlattenedFunction { .. } => {
            Ok(ControlFlow::Continue(Step::Into(true)))
        }
        SortExpressionKind::Reference(_) => unreachable!("Names must have been resolved"),
        _ => Ok(ControlFlow::Continue(Step::Into(observed))),
    })
    .map(|_| ())
}
