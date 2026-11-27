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
