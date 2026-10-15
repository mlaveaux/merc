//! Shared building blocks for a `TypingInfo`-accumulating walk over an
//! expression tree that lives *outside* the data specification proper — a
//! process body (`crate::process::check`), a PBES/PRES equation, or a modal formula.

use std::collections::HashMap;

use merc_syntax::ActDecl;
use merc_syntax::DataExpr;
use merc_syntax::IdDecl;
use merc_syntax::SortExpression;
use merc_syntax::Span;
use merc_syntax::VarId;

use crate::DataSpecification;
use crate::InferenceError;
use crate::ResolvedSortId;
use crate::Roots;
use crate::TypingInfo;
use crate::WellTypedError;
use crate::infer_expression_in_scope;
use crate::lower_data_expr;
use crate::typing_info;

/// Every declaration reachable from the `proc` body/PBES/PRES equation/modal formula currently
/// being checked — global variables, that declaration's own parameters, and every
/// `sum`/`dist`/quantifier binder anywhere in it — keyed by each declaration's own [VarId].
///
/// A flat list suffices rather than a stack of nested scopes: variable resolution already tied
/// every occurrence to its binder's unique [VarId], so a binder can only ever be found by the
/// occurrences it actually scopes over.
pub(crate) type Scope = [(VarId, ResolvedSortId)];

/// Prepares a raw expression for inference: resolves its embedded binder sorts (see
/// [`DataSpecification::resolve_expression_binder_sorts`]) and lowers it, exactly as
/// [`DataSpecification::typecheck_expression`] does for a standalone expression.
pub(crate) fn prepare_expression<E>(data: &mut DataSpecification, expr: &DataExpr) -> Result<DataExpr, E>
where
    E: From<WellTypedError>,
{
    let mut expr = expr.clone();
    data.resolve_expression_binder_sorts(&mut expr)?;
    Ok(lower_data_expr(expr))
}

/// Checks `expr` (a `sum`/`dist` condition or time bound, an assignment-form instantiation
/// argument, a `PropVarInst` argument, …) against `expected`, merging its `TypingInfo` into
/// `typing` on success.
///
/// Generic over the caller's own error type `E` (`ProcessError`, `PbesError`, …), which must
/// convert from [`WellTypedError`] and [`InferenceError`].
pub(crate) fn check_expression_against<E>(
    data: &mut DataSpecification,
    scope: &Scope,
    expr: &DataExpr,
    expected: ResolvedSortId,
    typing: &mut TypingInfo,
) -> Result<(), E>
where
    E: From<WellTypedError> + From<InferenceError>,
{
    let lowered = prepare_expression::<E>(data, expr)?;
    let (ctx, spec) = data.context_and_specs_mut();
    let equation_typing = infer_expression_in_scope(ctx, spec, &lowered, scope, Some(expected))?;
    typing.merge(typing_info::build(
        data,
        Roots::ExpressionAgainst {
            expr: &lowered,
            expected,
        },
        &equation_typing,
    ));
    Ok(())
}

/// Brings `variables`, whose sorts are already resolved to `sorts`, into `scope`, and records each
/// variable's own declaration occurrence in `typing` so it can be hovered/go-to-definition'd the
/// same as a use of it (see [`typing_info::push_binder_declaration`]).
pub(crate) fn declare_variables<'a>(
    data: &DataSpecification,
    scope: &mut Vec<(VarId, ResolvedSortId)>,
    typing: &mut TypingInfo,
    variables: impl IntoIterator<Item = &'a IdDecl>,
    sorts: impl IntoIterator<Item = ResolvedSortId>,
) {
    for (var, sort) in variables.into_iter().zip(sorts) {
        typing_info::push_binder_declaration(
            data,
            typing,
            var.identifier.span.clone(),
            var.identifier.node.clone(),
            sort,
        );
        let var_id = var.var_id.expect("variable resolution ran before checking");
        scope.push((var_id, sort));
    }
}

/// Resolves the sorts of the given binder variables with `resolve`, recording their sort
/// references, and declares them (see [`declare_variables`]).
pub(crate) fn collect_binder_sorts<E, F>(
    data: &mut DataSpecification,
    scope: &mut Vec<(VarId, ResolvedSortId)>,
    sort_references: &mut Vec<typing_info::SortReference>,
    typing: &mut TypingInfo,
    variables: &[IdDecl],
    mut resolve: F,
) -> Result<(), E>
where
    F: FnMut(&mut DataSpecification, &SortExpression) -> Result<ResolvedSortId, E>,
{
    let mut sorts = Vec::with_capacity(variables.len());
    for var in variables {
        typing_info::collect_sort_name_references(&var.sort, sort_references);
        sorts.push(resolve(data, &var.sort)?);
    }
    declare_variables(data, scope, typing, variables, sorts);
    Ok(())
}

/// Resolves a sort expression occurring in a declaration (an `act`/`proc`/`glob` parameter, a modal
/// fixpoint-variable parameter or binder, …): rejects an anonymous `struct` (never legal in a
/// declaration position), then defers to [`DataSpecification::resolve_declared_sort`] for the rest.
/// `anonymous_struct` builds the caller's own error variant for that rejection.
pub(crate) fn resolve_declared_sort<E, F>(
    data: &mut DataSpecification,
    sort: &SortExpression,
    anonymous_struct: F,
) -> Result<ResolvedSortId, E>
where
    E: From<WellTypedError>,
    F: FnOnce(Span) -> E,
{
    if let Some(span) = crate::find_anonymous_struct(sort) {
        return Err(anonymous_struct(span));
    }
    Ok(data.resolve_declared_sort(sort)?)
}

/// The resolved `act` declaration table, shared by `process`/`modal`: each declaration's resolved
/// argument-sort domain and declaring span, plus a name -> declaring-indices map for overload
/// resolution.
pub(crate) struct ActionTable {
    /// Resolved argument-sort domain of each action declaration, parallel to the `declarations`
    /// slice [`Self::build`] was given.
    pub(crate) action_domains: Vec<Vec<ResolvedSortId>>,
    /// Each declaration's own identifier span, parallel to `action_domains`.
    pub(crate) action_decl_spans: Vec<Span>,
    /// name -> indices into `action_domains`/`action_decl_spans` declaring it.
    pub(crate) actions_by_name: HashMap<String, Vec<usize>>,
}

impl ActionTable {
    /// Builds the table from every `act` declaration in `declarations`, resolving each one's
    /// argument sorts via `resolve_declared_sort`.
    ///
    /// An action's identity is its `(name, domain)` pair — there's no codomain for a second
    /// declaration to conflict on, so a repeat with the exact same name and domain as an earlier
    /// one is a harmless restatement, collapsed into the earlier entry the same way
    /// `signature::push_overload` collapses a repeated `map`/`cons` declaration. A different
    /// domain under the same name is a legitimate overload, kept as its own entry as always.
    pub(crate) fn build<E, F>(
        data: &mut DataSpecification,
        declarations: &[ActDecl],
        mut resolve_declared_sort: F,
    ) -> Result<Self, E>
    where
        F: FnMut(&mut DataSpecification, &SortExpression) -> Result<ResolvedSortId, E>,
    {
        let mut action_domains = Vec::with_capacity(declarations.len());
        let mut action_decl_spans = Vec::with_capacity(declarations.len());
        let mut actions_by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for decl in declarations {
            let domain = decl
                .args
                .iter()
                .map(|sort| resolve_declared_sort(data, sort))
                .collect::<Result<Vec<_>, _>>()?;

            let indices = actions_by_name.entry(decl.identifier.node.clone()).or_default();
            if indices.iter().any(|&i| action_domains[i] == domain) {
                continue;
            }

            indices.push(action_domains.len());

            action_domains.push(domain);
            action_decl_spans.push(decl.identifier.span.clone());
        }

        Ok(ActionTable {
            action_domains,
            action_decl_spans,
            actions_by_name,
        })
    }
}

/// Tries every candidate in `candidates`, each checked into its own scratch `TypingInfo` via
/// `check_candidate`, and requires exactly one to succeed — an mCRL2 overload set, the way
/// `crate::process::check::check_action_or_process`/`check_instantiation` and
/// `crate::modal::check::check_action` use this. A failed or ambiguous candidate's typing must never reach the caller's own
/// `TypingInfo`, so each candidate gets a fresh scratch one, returned to the caller only for the
/// single match.
///
/// `candidates` must be nonempty: the "no such name at all" case usually needs a different error
/// shape (e.g. every *other* declared name, as a suggestion), so callers check that separately.
pub(crate) fn resolve_single_candidate<C, E, F, G, H>(
    candidates: &[C],
    mut check_candidate: F,
    no_matching: G,
    ambiguous: H,
) -> Result<(&C, TypingInfo), E>
where
    F: FnMut(&C, &mut TypingInfo) -> Result<(), E>,
    G: FnOnce(E) -> E,
    H: FnOnce(usize) -> E,
{
    let mut successes = 0usize;
    let mut first_error = None;
    let mut matched: Option<(&C, TypingInfo)> = None;
    for candidate in candidates {
        let mut candidate_typing = TypingInfo::default();
        match check_candidate(candidate, &mut candidate_typing) {
            Ok(()) => {
                successes += 1;
                matched = Some((candidate, candidate_typing));
            }
            Err(error) => drop(first_error.get_or_insert(error)),
        }
    }

    match successes {
        0 => Err(no_matching(first_error.expect(
            "candidates is nonempty, so at least one recorded error when none succeed",
        ))),
        1 => Ok(matched.expect("successes == 1 implies a matched candidate")),
        count => Err(ambiguous(count)),
    }
}
