use rand::Rng;
use rand::RngExt;
use rand::seq::IndexedRandom;

use merc_utilities::Span;

use crate::BagElement;
use crate::ComplexSort;
use crate::DataExpr;
use crate::DataExprKind;
use crate::IdDecl;
use crate::Sort;
use crate::SortDecl;
use crate::SortExpression;
use crate::SortExpressionKind;
use crate::random_boolean_data_expression;
use crate::random_integer_data_expression;

fn id(identifier: String) -> DataExpr {
    DataExprKind::Id(identifier).into()
}

/// Resolves a `Reference`/`Resolved` sort name to its declared expression, if any.
fn resolve<'a>(sort_decls: &'a [SortDecl], name: &str) -> Option<&'a SortExpression> {
    sort_decls
        .iter()
        .find(|d| d.identifier == name)
        .and_then(|d| d.expr.as_ref())
}

/// Finds a variable name starting with `prefix` that does not collide with `freevars`.
fn fresh_name(freevars: &[IdDecl], prefix: &str) -> String {
    (0..)
        .map(|i| format!("{prefix}{i}"))
        .find(|name| !freevars.iter().any(|v| v.identifier.node == *name))
        .expect("infinite iterator")
}

/// Generates a random, well-typed value of the given sort, given the declared sorts a
/// [`crate::random_data_specification`] produced (so `Reference`/`Resolved` sort names can be
/// resolved) and the free variables in scope (reused directly when one has the exact same sort).
///
/// Every `SortExpressionKind` `random_data_specification` can produce is handled: basic sorts
/// fall back to [`random_boolean_data_expression`]/[`random_integer_data_expression`] (plus an
/// `Int2Real` conversion for `Real`), a `Struct` applies one of its constructors recursively, a
/// container `Complex` sort builds a small literal, and a `Function` sort builds a `lambda`
/// binding a fresh variable of the domain sort. Termination is guaranteed by
/// `random_data_specification`'s declared sorts being acyclic (a sort only ever references an
/// earlier one), not by `depth` -- `depth` only biases how eagerly an existing free variable of
/// the exact same sort is reused instead of building a fresh value.
pub fn random_value_expression<R: Rng>(
    rng: &mut R,
    sort_decls: &[SortDecl],
    sort: &SortExpression,
    freevars: &[IdDecl],
    depth: usize,
) -> DataExpr {
    if depth == 0 || rng.random_bool(0.3) {
        if let Some(v) = freevars.iter().find(|v| &v.sort == sort) {
            return id(v.identifier.node.clone());
        }
    }

    match &sort.node {
        SortExpressionKind::Simple(Sort::Bool) => random_boolean_data_expression(rng, freevars),
        // `random_integer_data_expression` can produce `0` or a `Subtract`, neither of which is
        // a valid `Pos` value (unlike `Nat`/`Int`, `Pos` excludes zero and has no unary minus),
        // so `Pos` needs its own, narrower generator rather than reusing it directly.
        SortExpressionKind::Simple(Sort::Pos) => {
            let positives: Vec<&IdDecl> = freevars
                .iter()
                .filter(|v| matches!(&v.sort.node, SortExpressionKind::Simple(Sort::Pos)))
                .collect();
            if !positives.is_empty() && rng.random_bool(0.5) {
                id(positives
                    .choose(rng)
                    .expect("positives is non-empty")
                    .identifier
                    .node
                    .clone())
            } else {
                DataExprKind::Number(["1", "2", "3"].choose(rng).expect("non-empty").to_string()).into()
            }
        }
        SortExpressionKind::Simple(Sort::Int) => random_integer_data_expression(rng, freevars),
        // `random_integer_data_expression`'s candidates include `Subtract`, which (like any
        // Int-producing operation) does not always stay within `Nat`/`Real`; wrap with the
        // matching conversion function rather than risk emitting e.g. `(m - n): Int` where a
        // `Nat`/`Real` value is required.
        SortExpressionKind::Simple(Sort::Nat) => DataExprKind::Application {
            function: Box::new(id("Int2Nat".to_string())),
            arguments: vec![random_integer_data_expression(rng, freevars)],
        }
        .into(),
        SortExpressionKind::Simple(Sort::Real) => DataExprKind::Application {
            function: Box::new(id("Int2Real".to_string())),
            arguments: vec![random_integer_data_expression(rng, freevars)],
        }
        .into(),
        SortExpressionKind::Reference(name) | SortExpressionKind::Resolved(name, _) => {
            match resolve(sort_decls, name) {
                Some(inner) => random_value_expression(rng, sort_decls, inner, freevars, depth),
                None => freevars
                    .iter()
                    .find(|v| &v.sort == sort)
                    .map(|v| id(v.identifier.node.clone()))
                    .unwrap_or_else(|| panic!("no way to construct a value of uninterpreted sort {sort}")),
            }
        }
        SortExpressionKind::Struct { inner } => {
            let ctor = inner
                .choose(rng)
                .expect("a generated struct sort always has at least one constructor");
            let args: Vec<DataExpr> = ctor
                .args
                .iter()
                .map(|(_, field_sort)| {
                    random_value_expression(rng, sort_decls, field_sort, freevars, depth.saturating_sub(1))
                })
                .collect();
            if args.is_empty() {
                id(ctor.name.node.clone())
            } else {
                DataExprKind::Application {
                    function: Box::new(id(ctor.name.node.clone())),
                    arguments: args,
                }
                .into()
            }
        }
        SortExpressionKind::Complex(container, element) => {
            let count = rng.random_range(0..=2usize);
            let elements: Vec<DataExpr> = (0..count)
                .map(|_| random_value_expression(rng, sort_decls, element, freevars, depth.saturating_sub(1)))
                .collect();
            match container {
                ComplexSort::List => {
                    if elements.is_empty() {
                        DataExprKind::EmptyList.into()
                    } else {
                        DataExprKind::List(elements).into()
                    }
                }
                ComplexSort::Set | ComplexSort::FSet => {
                    if elements.is_empty() {
                        DataExprKind::EmptySet.into()
                    } else {
                        DataExprKind::Set(elements).into()
                    }
                }
                ComplexSort::Bag | ComplexSort::FBag => {
                    if elements.is_empty() {
                        DataExprKind::EmptyBag.into()
                    } else {
                        let bag_elements = elements
                            .into_iter()
                            .map(|expr| BagElement {
                                expr,
                                multiplicity: DataExprKind::Number("1".to_string()).into(),
                            })
                            .collect();
                        DataExprKind::Bag(bag_elements).into()
                    }
                }
            }
        }
        SortExpressionKind::Function { domain, range } => {
            let var_name = fresh_name(freevars, "lv");
            let var_decl = IdDecl::new(var_name, (**domain).clone(), Span::default());
            let mut inner_freevars = freevars.to_vec();
            inner_freevars.push(var_decl.clone());
            let body = random_value_expression(rng, sort_decls, range, &inner_freevars, depth.saturating_sub(1));
            DataExprKind::Lambda {
                variables: vec![var_decl],
                body: Box::new(body),
            }
            .into()
        }
        // random_data_specification never generates Product/TypeVar/ResolvedTypeVar/FlattenedFunction.
        other => unreachable!("unexpected sort shape from a generated data specification: {other:?}"),
    }
}
