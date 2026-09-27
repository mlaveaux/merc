use std::collections::HashMap;
use std::convert::Infallible;

use log::debug;

use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::SortId;
use merc_syntax::Spanned;
use merc_syntax::Traverse;
use merc_syntax::UntypedDataSpecification;

use crate::apply_sorts_in_spec;

/// Normalizes every sort in `spec` to a canonical form by expanding aliases.
///
/// A non-structured alias (`sort D = Nat;`, `sort L = List(D);`) is replaced by
/// its recursively normalized definition, so an alias and the sort it stands
/// for become indistinguishable and sort equality is structural. A
/// structured-sort alias is instead its own representative and keeps its name,
/// because structured sorts are identified by name and because expanding a
/// recursive `struct` would not terminate.
///
/// Terminates on every specification that
/// [`check_aliases`](super::alias::check_aliases) accepts.
pub(crate) fn normalize_sorts(spec: &mut UntypedDataSpecification) {
    // Clone the alias right-hand sides so the rewrite can borrow `spec` mutably
    // while still consulting the alias map.
    let alias_map: HashMap<SortId, SortExpression> = spec
        .sort_declarations
        .iter()
        .filter_map(|decl| Some((decl.id.expect("Name must have been resolved"), decl.expr.clone()?)))
        .collect();

    apply_sorts_in_spec(spec, |sort| -> Result<_, Infallible> {
        let result = normalize_sort(sort, &alias_map, &mut Vec::new());

        if result != *sort {
            debug!("normalize: sort '{sort}' expanded to '{result}'");
        }

        Ok(result)
    })
    .expect("normalization never fails");
}

/// Recursively normalizes a single sort against the alias map. `visited` holds
/// the aliases currently being expanded, so an alias reached again is kept as a
/// named representative instead of being unfolded forever.
fn normalize_sort(
    sort: &SortExpression,
    alias_map: &HashMap<SortId, SortExpression>,
    visited: &mut Vec<SortId>,
) -> SortExpression {
    sort.clone()
        .apply(|expr| -> Result<_, Infallible> {
            let SortExpressionKind::Resolved(_, id) = &expr.node else {
                return Ok(None);
            };

            // A structured-sort alias, an abstract sort, or an alias reached again
            // while it is being expanded, is a named representative: keep the name
            // and do not recurse, so recursion through a `struct` terminates.
            if visited.contains(id) {
                return Ok(None);
            }
            match alias_map.get(id) {
                Some(Spanned {
                    node: SortExpressionKind::Struct { .. },
                    ..
                })
                | None => Ok(None),
                Some(alias) => {
                    visited.push(*id);
                    let result = normalize_sort(alias, alias_map, visited);
                    visited.pop();
                    Ok(Some(result))
                }
            }
        })
        .expect("normalization never fails")
}

#[cfg(test)]
mod tests {
    use merc_syntax::Sort;
    use merc_syntax::SortExpression;
    use merc_syntax::SortExpressionKind;
    use merc_syntax::UntypedDataSpecification;

    use crate::DataSpecification;

    /// Type checks `text` and returns the (normalized) sort of the map `name`.
    fn map_sort(text: &str, name: &str) -> SortExpression {
        let spec = DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()).unwrap();
        spec.data_specification()
            .map_declarations
            .iter()
            .find(|map| map.identifier.node == name)
            .unwrap_or_else(|| panic!("map {name} should be declared"))
            .sort
            .clone()
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_alias_to_basic_sort_is_expanded() {
        // `D` aliases `Nat`, so `f: D` normalizes to the built-in `Nat` sort.
        let sort = map_sort("sort D = Nat; map f: D;", "f");
        assert_eq!(sort.node, SortExpressionKind::Simple(Sort::Nat));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_alias_chain_is_expanded() {
        let sort = map_sort("sort D = Nat; E = D; map f: E;", "f");
        assert_eq!(sort.node, SortExpressionKind::Simple(Sort::Nat));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_alias_inside_container_is_expanded() {
        // `f: List(D)` with `D = Nat` normalizes to `List(Nat)`.
        let sort = map_sort("sort D = Nat; map f: List(D);", "f");
        let SortExpressionKind::Complex(op, subsort) = sort.into_node() else {
            panic!("expected a container sort");
        };
        assert_eq!(op, merc_syntax::ComplexSort::List);
        assert_eq!(subsort.into_node(), SortExpressionKind::Simple(Sort::Nat));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_structured_alias_keeps_its_name() {
        // A structured sort is its own representative, so `f: D` stays `D`
        // rather than being replaced by the (recursive) struct body.
        let sort = map_sort("sort D = struct a | b; map f: D;", "f");
        let SortExpressionKind::Resolved(name, _) = sort.into_node() else {
            panic!("expected a resolved nominal sort");
        };
        assert_eq!(name, "D");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_chained_struct_alias_shares_representative() {
        // `A = B` chains to the structured sort `B`, so `f: A` and `g: B`
        // normalize to the same named representative rather than diverging.
        let text = "sort A = B; B = struct c; map f: A; g: B;";
        let a = map_sort(text, "f");
        let b = map_sort(text, "g");
        assert_eq!(a, b);
        let SortExpressionKind::Resolved(name, _) = a.into_node() else {
            panic!("expected a resolved nominal sort");
        };
        assert_eq!(name, "B");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_recursive_alias_through_inline_struct_terminates() {
        // `D` recurses into itself through an inline `struct`, which
        // check_aliases permits (it stops at every struct); normalization must
        // keep the back-reference named rather than unfold it forever.
        let sort = map_sort("sort D = List(struct f(D)); map g: D;", "g");
        let SortExpressionKind::Complex(op, _) = sort.node else {
            panic!("expected a List container");
        };
        assert_eq!(op, merc_syntax::ComplexSort::List);
    }
}
