use merc_syntax::ConstructorId;
use merc_syntax::MapId;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::SortId;
use merc_syntax::UntypedDataSpecification;
use merc_syntax::VarId;

use crate::EquationRole;
use crate::ResolvedSortId;
use crate::TypeCheckContext;

/// Returns the resolved sort of the constructor with the given [ConstructorId].
///
/// Covers the user specification only; the system-defined specification is
/// still unresolved content.
pub(crate) fn query_sort_of_constructor(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    id: ConstructorId,
) -> ResolvedSortId {
    if let Some(value) = ctx.sort_of_constructor.get(&id) {
        return *value;
    }
    let value = resolve_sort(ctx, spec, &spec.constructor_declarations[id].sort);
    ctx.sort_of_constructor.insert(id, value);
    value
}

/// Returns the resolved sort of the map with the given [MapId], memoized on
/// [TypeCheckContext::sort_of_map]. Requires `id` to originate from
/// `assign_declaration_ids` on `spec`.
///
/// Covers the user specification only; the system-defined specification is
/// still unresolved content.
pub(crate) fn query_sort_of_map(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    id: MapId,
) -> ResolvedSortId {
    if let Some(value) = ctx.sort_of_map.get(&id) {
        return *value;
    }
    let value = resolve_sort(ctx, spec, &spec.map_declarations[id].sort);
    ctx.sort_of_map.insert(id, value);
    value
}

/// Returns the resolved sort of the equation `var`-block variable declared by `sort`, identified
/// by its own `var_id`.
pub(crate) fn query_sort_of_equation_var(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    role: EquationRole,
    var_id: VarId,
    sort: &SortExpression,
) -> ResolvedSortId {
    if let Some(value) = ctx.sort_of_equation_var.get(&(role, var_id)) {
        return *value;
    }

    let value = resolve_sort(ctx, spec, sort);
    ctx.sort_of_equation_var.insert((role, var_id), value);
    value
}

/// Resolves a sort expression into its corresponding interned sort identifier.
///
/// Requires names resolved and structured sorts desugared, and that alias are
/// already normalised.
pub(crate) fn resolve_sort(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    sort: &SortExpression,
) -> ResolvedSortId {
    match &sort.node {
        SortExpressionKind::Simple(sort) => ctx.sorts.primitive(*sort),
        SortExpressionKind::Complex(op, subsort) => {
            let subsort = resolve_sort(ctx, spec, subsort);
            ctx.sorts.generic(*op, subsort)
        }
        SortExpressionKind::FlattenedFunction { domain, range } => {
            let domain = domain.iter().map(|sort| resolve_sort(ctx, spec, sort)).collect();
            let range = resolve_sort(ctx, spec, range);
            ctx.sorts.function(domain, range)
        }
        // Kept so the resolver accepts any well-formed sort expression, such
        // as binder sorts built during inference.
        SortExpressionKind::Function { domain, range } => {
            let mut resolved_domain = Vec::new();
            resolve_function_domain(ctx, spec, domain, &mut resolved_domain);
            let range = resolve_sort(ctx, spec, range);
            ctx.sorts.function(resolved_domain, range)
        }
        SortExpressionKind::Resolved(_, id) => query_sort_of_def(ctx, spec, *id),
        SortExpressionKind::Reference(_) => unreachable!("Names must have been resolved"),
        SortExpressionKind::TypeVar(_) => unreachable!("Names must have been resolved"),
        // Interned as a genuine lattice element, the same `S` is considered
        // identical wherever it appears.
        SortExpressionKind::ResolvedTypeVar(id) => ctx.sorts.var(*id),
        SortExpressionKind::Struct { .. } => unreachable!("Structured sorts must have been desugared"),
        SortExpressionKind::Product { .. } => {
            unreachable!("product sorts outside a function domain were rejected before resolution")
        }
    }
}

/// Resolves the leaves of a `Product` domain spine in declaration order, the
/// resolution counterpart of `flatten_function_domain_rec`.
fn resolve_function_domain(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    sort: &SortExpression,
    domain: &mut Vec<ResolvedSortId>,
) {
    match &sort.node {
        SortExpressionKind::Product { lhs, rhs } => {
            resolve_function_domain(ctx, spec, lhs, domain);
            resolve_function_domain(ctx, spec, rhs, domain);
        }
        _ => domain.push(resolve_sort(ctx, spec, sort)),
    }
}

/// Returns the resolved sort denoted by a sort declaration: the nominal sort
/// for an abstract sort or struct representative (no alias body), or the
/// resolved body for an alias. Memoized on [TypeCheckContext::sort_of_def].
///
/// Requires `def` to originate from name resolution of `spec`, so it indexes
/// `sort_declarations`. Cyclic aliases were rejected by `check_aliases`, so the
/// query cannot re-enter itself, whether alias bodies are normalized or not.
pub(crate) fn query_sort_of_def(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    def: SortId,
) -> ResolvedSortId {
    debug_assert!(
        spec.sort_declarations
            .get(*def)
            .is_some_and(|decl| decl.id == Some(def)),
        "SortId {def:?} does not originate from name resolution of this specification"
    );

    if let Some(value) = ctx.sort_of_def.get(&def) {
        return *value;
    }
    let value = match &spec.sort_declarations[*def].expr {
        None => ctx.sorts.def(def),
        Some(expr) => resolve_sort(ctx, spec, expr),
    };
    ctx.sort_of_def.insert(def, value);
    value
}
