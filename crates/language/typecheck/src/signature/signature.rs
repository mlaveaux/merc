use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::Span;
use merc_syntax::UntypedDataSpecification;

use crate::BUILTIN_SCHEME_TEMPLATE;
use crate::CONTAINER_TEMPLATES;
use crate::DisplaySortContext;
use crate::ResolvedSort;
use crate::ResolvedSortId;
use crate::TypeCheckContext;
use crate::WellTypedError;
use crate::build_polymorphic_schemes;
use crate::check_products_within_domains;
use crate::query_sort_of_constructor;
use crate::query_sort_of_map;
use crate::resolve_sort;

/// A polymorphic overload: `sort` is a [ResolvedSortId] built by
/// [`resolve_sort`] from a template's own declaration.
#[derive(Clone, Debug)]
pub(crate) struct PolySortScheme {
    pub(crate) sort: ResolvedSortId,
}

/// The (S, C, M) signature of a specification (Definition 15.1.5): the resolved
/// overload set of every constructor and mapping name, the lookup table for
/// overload resolution.
///
/// A symbol is a name together with its sort, so a name maps to one
/// [ResolvedSortId] per overload; duplicate declarations of the same symbol
/// collapse into one entry.
#[derive(Default)]
pub(crate) struct Signature {
    pub(crate) constructors: HashMap<String, Vec<ResolvedSortId>>,
    pub(crate) mappings: HashMap<String, Vec<ResolvedSortId>>,
    pub(crate) schemes: HashMap<String, Vec<PolySortScheme>>,
}

impl Signature {
    /// Renders `self` with every overload's [`ResolvedSortId`] resolved to its
    /// printable sort (via [`DisplaySortContext`]), sorted by name so the
    /// output is deterministic.
    pub(crate) fn display<'a>(
        &'a self,
        ctx: &'a TypeCheckContext,
        spec: &'a UntypedDataSpecification,
    ) -> DisplaySignature<'a> {
        DisplaySignature {
            signature: self,
            ctx,
            spec,
        }
    }
}

/// See [`Signature::display`].
pub(crate) struct DisplaySignature<'a> {
    signature: &'a Signature,
    ctx: &'a TypeCheckContext,
    spec: &'a UntypedDataSpecification,
}

impl fmt::Display for DisplaySignature<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let write_overloads =
            |f: &mut fmt::Formatter<'_>, keyword: &str, overloads: &HashMap<String, Vec<ResolvedSortId>>| {
                if overloads.is_empty() {
                    return Ok(());
                }

                let mut names: Vec<&String> = overloads.keys().collect();
                names.sort();

                writeln!(f, "{keyword}")?;
                for name in names {
                    for &sort in &overloads[name] {
                        writeln!(f, "   {name}: {};", DisplaySortContext::new(self.ctx, self.spec, sort))?;
                    }
                }
                Ok(())
            };

        write_overloads(f, "cons", &self.signature.constructors)?;
        write_overloads(f, "map", &self.signature.mappings)?;

        if !self.signature.schemes.is_empty() {
            let mut names: Vec<&String> = self.signature.schemes.keys().collect();
            names.sort();

            writeln!(f, "schemes")?;
            for name in names {
                for scheme in &self.signature.schemes[name] {
                    writeln!(
                        f,
                        "   {name}: {};",
                        DisplaySortContext::new(self.ctx, self.spec, scheme.sort)
                    )?;
                }
            }
        }

        Ok(())
    }
}

/// Computes the signature of `spec` and stores it on `ctx`, running the
/// signature-layer well-typedness checks of 15.1.7. Idempotent: a second call
/// is a no-op that returns the already-stored result.
///
/// Runs *before* alias normalization, so the errors refer to sorts as the user
/// wrote them (`D` rather than its alias expansion `Nat`).
///
/// # Requirements
///
/// Requires names to be resolved and structured sorts to be desugared.
pub(crate) fn build_signature<'a>(
    ctx: &'a mut TypeCheckContext,
    spec: &UntypedDataSpecification,
) -> Result<&'a Signature, WellTypedError> {
    if ctx.signature.is_none() {
        let signature = compute_signature(ctx, spec)?;
        ctx.signature = Some(Arc::new(signature));
    }

    Ok(ctx.signature.as_deref().expect("the signature was just computed"))
}

fn compute_signature(ctx: &mut TypeCheckContext, spec: &UntypedDataSpecification) -> Result<Signature, WellTypedError> {
    let mut signature = Signature::default();
    let mut constants: HashMap<String, ResolvedSortId> = HashMap::new();
    push_declarations(ctx, spec, spec, false, &mut signature, &mut constants)?;

    // The polymorphic built-ins.
    signature.schemes = build_polymorphic_schemes(
        ctx,
        CONTAINER_TEMPLATES.all().into_iter().chain([&*BUILTIN_SCHEME_TEMPLATE]),
    );

    Ok(signature)
}

/// Checks and collects `decl_spec`'s own constructor/mapping declarations into `signature`,
/// running every signature-level well-typedness rule of Definition 15.1.5/15.1.7 `is_well_typed`
/// doesn't already cover post-normalization:
///
/// - no product sort outside a function domain,
/// - no constructor for a function sort,
/// - constructor/mapping disjointness, and
/// - no zero-arity symbol declared twice under different sorts — the shared `constants` table
///   means this holds across both declaration kinds and, when called again for a second spec,
///   across that call too (see `resolve_system_signature`).
///
/// `resolve_spec` is the specification whose `sort_declarations` table a `Resolved(name, SortId)`
/// node in `decl_spec` indexes into.
///
/// `trusted` skips the one rule the system-defined specification's own basic-sort constructors
/// (`@c0: Nat`, `@cNat`, ...) legitimately break: no constructor for a basic sort.
pub(crate) fn push_declarations(
    ctx: &mut TypeCheckContext,
    decl_spec: &UntypedDataSpecification,
    resolve_spec: &UntypedDataSpecification,
    trusted: bool,
    signature: &mut Signature,
    constants: &mut HashMap<String, ResolvedSortId>,
) -> Result<(), WellTypedError> {
    // resolve_sort has no meaning for (and panics on) a product sort outside a
    // function domain, so every sort this query resolves is checked first.
    for sort in decl_spec
        .sort_declarations
        .iter()
        .filter_map(|decl| decl.expr.as_ref())
        .chain(decl_spec.constructor_declarations.iter().map(|decl| &decl.sort))
        .chain(decl_spec.map_declarations.iter().map(|decl| &decl.sort))
    {
        check_products_within_domains(sort)?;
    }

    for decl in &decl_spec.constructor_declarations {
        // Resolve through the memoized, `ConstructorId`-keyed query for the user's own
        // specification, so lowering can later read the interned constructor sort straight from
        // the context. `trusted` content resolves directly against `resolve_spec` instead.
        let sort_id = if trusted {
            resolve_sort(ctx, resolve_spec, &decl.sort)
        } else {
            let constructor_id = decl.id.expect("assign_declaration_ids ran before build_signature");
            query_sort_of_constructor(ctx, decl_spec, constructor_id)
        };

        // The constructor targets the range of its (function) sort. The check
        // is semantic — an alias of `Nat` is rejected like `Nat` itself — but
        // the error reports the target as written. When the whole constructor
        // sort is an alias of a function sort, the written sort itself is the
        // closest the user came to writing the target.
        let target = match ctx.sorts.get(sort_id) {
            ResolvedSort::Function { domain: _, range } => *range,
            _ => sort_id,
        };
        match ctx.sorts.get(target) {
            ResolvedSort::Primitive(_) if !trusted => {
                return Err(WellTypedError::ConstructorForBasicSort {
                    constructor: decl.identifier.node.clone(),
                    sort: written_target_sort(&decl.sort).to_string(),
                    span: decl.identifier.span.clone(),
                });
            }
            ResolvedSort::Function { .. } => {
                return Err(WellTypedError::ConstructorForFunctionSort {
                    constructor: decl.identifier.node.clone(),
                    sort: written_target_sort(&decl.sort).to_string(),
                    span: decl.identifier.span.clone(),
                });
            }
            _ => {}
        }

        check_constant_name(constants, ctx, &decl.identifier, decl.identifier.span.clone(), sort_id)?;
        push_overload(
            signature.constructors.entry(decl.identifier.node.clone()).or_default(),
            sort_id,
        );
    }

    for decl in &decl_spec.map_declarations {
        let id = if trusted {
            resolve_sort(ctx, resolve_spec, &decl.sort)
        } else {
            let map_id = decl.id.expect("assign_declaration_ids ran before build_signature");
            query_sort_of_map(ctx, decl_spec, map_id)
        };

        // The constructors and mappings must be disjoint *as symbols*: the same
        // name under both `cons` and `map` conflicts exactly when the resolved
        // sorts coincide, which also catches sorts that only differ through an
        // alias. Overloading the name with a different sort remains allowed.
        if signature
            .constructors
            .get(&decl.identifier.node)
            .is_some_and(|overloads| overloads.contains(&id))
        {
            return Err(WellTypedError::ConstructorAndMappingConflict {
                constructor: decl.identifier.node.clone(),
                map: decl.identifier.node.clone(),
                span: decl.identifier.span.clone(),
            });
        }

        check_constant_name(constants, ctx, &decl.identifier, decl.identifier.span.clone(), id)?;
        push_overload(signature.mappings.entry(decl.identifier.node.clone()).or_default(), id);
    }

    Ok(())
}

/// The range of a written (function) sort, or the sort itself otherwise — for rendering the
/// "sort as written" half of a [`WellTypedError::ConstructorForBasicSort`]/
/// [`WellTypedError::ConstructorForFunctionSort`] message.
///
/// Unlike [`crate::target_sort`], this tolerates a plain `Function` node, not just `FlattenedFunction`:
/// the user's own declarations are always already flattened by the time `push_declarations` sees
/// them, but a `trusted` specification's are not (`resolve_system_signature` never runs
/// `flatten_function_sorts` over `system`/`basics` — nothing needed it to, since no real system
/// content has ever hit this error path before). Asserting the precondition here, the way
/// `target_sort` does, would turn a `trusted` equation's *rejection* into a panic instead of the
/// `WellTypedError` this whole check exists to produce in the first place.
fn written_target_sort(sort: &SortExpression) -> &SortExpression {
    match &sort.node {
        SortExpressionKind::Function { range, .. } | SortExpressionKind::FlattenedFunction { range, .. } => range,
        _ => sort,
    }
}

/// Rejects a second zero-arity declaration of `name` under a different sort
/// than a previous one (see the comment on `constants` in
/// [compute_signature]). Symbols with a function sort are not zero-arity and
/// pass through untouched.
fn check_constant_name(
    constants: &mut HashMap<String, ResolvedSortId>,
    ctx: &TypeCheckContext,
    name: &str,
    span: Span,
    id: ResolvedSortId,
) -> Result<(), WellTypedError> {
    if matches!(ctx.sorts.get(id), ResolvedSort::Function { .. }) {
        return Ok(());
    }
    match constants.get(name) {
        Some(&existing) if existing != id => Err(WellTypedError::DuplicateConstantDifferentSort {
            name: name.to_string(),
            span,
        }),
        _ => {
            constants.insert(name.to_string(), id);
            Ok(())
        }
    }
}

/// Appends `id` unless it is already an overload, so duplicate declarations of
/// the same symbol collapse into one entry.
pub(crate) fn push_overload(overloads: &mut Vec<ResolvedSortId>, id: ResolvedSortId) {
    if !overloads.contains(&id) {
        overloads.push(id);
    }
}
