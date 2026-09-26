// Monomorphizes system-defined (Appendix-B) equations for lowering: turns each container/
// function-update/comparison template instantiation the specification actually uses into ground
// content by substituting into the template's own already-proven, rigid typing — never by
// inferring it again. See `TemplateInstantiation`'s own doc comment for why substitution is what
// makes two instantiations of the same template (`Bag(Nat)`, `Bag(D)`) sound without re-checking
// either one, and `crate::check_system_equations`'s doc comment for the one kind of system
// equation this does *not* cover (`system`'s own basics/desugared-struct equations, which are
// genuinely inferred, once).
//
// This is also where the worklist that discovers *which* instantiations a specification needs
// lives (`build_system_defined_specification`/`extend_system_with_inferred_sorts`/`expand_sorts`):
// stamping out concrete sorts is instantiation-level work, not a type-checking concern — at the
// type-checking level (`crate::signature`) only the templates' own generic equations are checked,
// once, rigidly, with no concrete sort in sight (`check_container_templates`/
// `check_comparison_template`/`check_function_update_template`).

use std::collections::HashMap;
use std::collections::HashSet;
use std::convert::Infallible;
use std::ops::ControlFlow;
use std::ops::Range;
use std::sync::Arc;

use merc_syntax::ComplexSort;
use merc_syntax::DataExpr;
use merc_syntax::DataExprKind;
use merc_syntax::EqnSpec;
use merc_syntax::OffsetSpans;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::SourceMap;
use merc_syntax::Traverse;
use merc_syntax::TypeVarId;
use merc_syntax::UntypedDataSpecification;

use crate::BUILTIN_SCHEME_TEMPLATE;
use crate::BUILTIN_SCHEME_TEMPLATE_TEXT;
use crate::EquationTyping;
use crate::ExprId;
use crate::NameTarget;
use crate::NumberEncoding;
use crate::ResolvedSort;
use crate::ResolvedSortId;
use crate::TemplateId;
use crate::TypeCheckContext;
use crate::apply_sorts_in_spec;
use crate::basic_sort_own_template;
use crate::container_templates;
use crate::function_update_template;
use crate::function_update_text;
use crate::is_supported_binder_sort;
use crate::lower_data_expressions;
use crate::register_bare_template;
use crate::resolve_sort;

/// Which template produced one contiguous range of
/// `system.equation_declarations` (`EqnSpecId` block indices), and the concrete
/// sort(s) substituted for that template's own `type_var` declaration(s), in
/// declaration order.
///
/// Used to specialize each generated equation's typing from the template's own
/// proven, rigid typing (`ctx.template_typings`) by substitution, instead of
/// re-checking it: two instantiations of the same container template
/// (`Bag(Nat)`, `Bag(D)`) each carry a copy of its equations, checked once as
/// the template's own.
pub(crate) struct TemplateInstantiation {
    pub(crate) template: TemplateId,
    pub(crate) substitution: Vec<SortExpression>,
    pub(crate) equation_range: Range<usize>,
}

/// Which sort-expression nodes are being collected.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortCollectionMode {
    /// Container sorts only — used to re-scan already-generated container
    /// content, where a growing function sort like
    /// `@is_not_an_update: (S -> T) -> Bool` would otherwise diverge (see
    /// [collect_system_sorts]'s doc comment).
    ContainersOnly,
    /// Container sorts and single-/multi-argument function sorts — used to
    /// seed the container worklist from the user's own specification.
    ContainersAndFunctions,
    /// Every sort — used to seed and re-scan the comparison-operator
    /// worklist: `==`/`<`/`if` apply uniformly to any sort, not just
    /// containers and functions.
    Every,
}

/// Drains `worklist` to a fixpoint like [expand_sorts], merging each popped
/// sort's generated batch into `result` directly. Unlike an earlier version
/// of this function, batches are no longer partitioned by element sort before
/// merging: the container/function-update/comparison operations are looked up
/// as schemes in one pooled signature regardless of which concrete
/// instantiation an equation came from, so there is nothing left for two
/// instantiations' equations to collide over. Records a [TemplateInstantiation]
/// for each batch, so its equations can be specialized from the template's
/// own proven typing rather than re-checked.
fn merge_generated(
    sources: &mut SourceMap,
    result: &mut UntypedDataSpecification,
    worklist: Vec<SortExpression>,
    seen: &HashSet<SortExpression>,
    scan_mode: SortCollectionMode,
    generate: impl FnMut(&mut SourceMap, &SortExpression) -> (UntypedDataSpecification, (TemplateId, Vec<SortExpression>)),
    extra_dependencies: impl FnMut(&SortExpression) -> Vec<SortExpression>,
) -> Vec<TemplateInstantiation> {
    let mut seen = seen.clone();
    let mut instantiations = Vec::new();
    expand_sorts(
        sources,
        worklist,
        &mut seen,
        scan_mode,
        generate,
        extra_dependencies,
        |mut generated, (template, substitution)| {
            lower_data_expressions(&mut generated);

            let start = result.equation_declarations.len();
            result.merge(&generated);
            let end = result.equation_declarations.len();

            instantiations.push(TemplateInstantiation {
                template,
                substitution,
                equation_range: start..end,
            });
        },
    );
    instantiations
}

/// Constructs a data specification for a standard sort, in the given
/// `encoding`, registered into `sources`, alongside which [TemplateId]
/// (bundled or generic) produced the result and the concrete sort(s)
/// substituted for its `type_var` declaration(s), in declaration order — used
/// by [merge_generated] to record a [TemplateInstantiation] for later
/// specialization instead of re-checking each generated equation from
/// scratch. A caller uninterested in the provenance (the `container_seen`
/// reconstruction pass below) just discards the second element, the same way
/// [comparison_operator_equations]'s own callers do.
pub(crate) fn standard_sort(
    sources: &mut SourceMap,
    sort: &SortExpression,
    encoding: NumberEncoding,
) -> (UntypedDataSpecification, (TemplateId, Vec<SortExpression>)) {
    let templates = container_templates(sources, encoding);

    if let SortExpressionKind::Complex(complex, element) = &sort.node {
        let (id, template) = match complex {
            ComplexSort::List => (TemplateId::List, &templates.list),
            ComplexSort::Set => (TemplateId::Set, &templates.set),
            ComplexSort::FSet => (TemplateId::FSet, &templates.fset),
            ComplexSort::Bag => (TemplateId::Bag, &templates.bag),
            ComplexSort::FBag => (TemplateId::FBag, &templates.fbag),
        };

        (replace_sort(template, "S", element), (id, vec![(**element).clone()]))
    } else if let SortExpressionKind::Function { domain, range } = &sort.node {
        // A single-argument function sort: generated the same way as a
        // multi-argument one, just with a one-element domain — see
        // [function_update].
        (
            function_update(sources, std::slice::from_ref(domain), range),
            (
                TemplateId::FunctionUpdateN(1),
                vec![(**domain).clone(), (**range).clone()],
            ),
        )
    } else if let SortExpressionKind::FlattenedFunction { domain, range } = &sort.node {
        // A multi-argument function sort: its own generic, arity-parameterized
        // template is built (and, once per arity, checked) separately — see
        // [function_update].
        let arity = domain.len();
        let mut substitution = domain.clone();
        substitution.push((**range).clone());
        (
            function_update(sources, domain, range),
            (TemplateId::FunctionUpdateN(arity), substitution),
        )
    } else {
        unreachable!("The given sort {} is not a standard sort", sort);
    }
}

/// Generates the function-update operators (`@func_update`,
/// `@func_update_stable`, `@is_not_an_update`, `@if_always_else`, Appendix
/// B.11) for a function sort of arity `domain.len() >= 1` — single- and
/// multi-argument functions alike — over the flattened domain
/// `D_0 # ... # D_{n-1} -> T`, by substituting the concrete domain/range
/// sorts into [function_update_template]'s generic, arity-matched template,
/// exactly like [standard_sort]'s own substitution of a concrete element sort
/// into a bundled container template.
fn function_update(
    sources: &mut SourceMap,
    domain: &[SortExpression],
    range: &SortExpression,
) -> UntypedDataSpecification {
    let arity = domain.len();
    let template = function_update_template(arity);
    let mut spec = template;
    for (i, argument_sort) in domain.iter().enumerate() {
        spec = replace_sort(&spec, &format!("S{i}"), argument_sort);
    }
    let mut spec = replace_sort(&spec, "T", range);

    // Registers a rendering of this concrete instantiation, purely for error
    // display: `spec`'s own spans still point at the *template*'s content,
    // which — since only the domain/range sort text was substituted — reads
    // identically to this rendering, so offsetting them into it is exact.
    let domain_sorts = domain
        .iter()
        .map(SortExpression::to_string)
        .collect::<Vec<_>>()
        .join(" # ");
    let domain_names: Vec<String> = domain.iter().map(SortExpression::to_string).collect();
    let text = function_update_text(&domain_names, &range.to_string());
    let id = sources.add_virtual(
        format!("<generated>/function_update({domain_sorts} -> {range}).mcrl2"),
        text,
    );
    let base = sources.base_offset(id);
    spec.offset_spans(base);
    spec
}

/// As [standard_sort], but instantiates the reflexive/derived
/// comparison-operator equations (`crate::BUILTIN_SCHEME_TEMPLATE`'s own
/// `eqn` block) for `sort` instead of a container/function-update template —
/// applies uniformly to any concrete sort, since the template holds only one
/// `type_var S` and no branching on `sort`'s shape.
///
/// Registers a fresh virtual document per call, the same way
/// `container_templates_binary`/`container_templates_machine_word` do for a
/// bundled container template: `BUILTIN_SCHEME_TEMPLATE` itself is parsed
/// once with no `SourceMap` involved (see `crate::parse_template_bare`), so
/// without this its spans would render against nothing.
///
/// Only equations are returned; the `map` signatures are dropped after
/// substitution, deliberately. Unlike a container operation (`in`, `count`,
/// …), `==`/`<`/`if` are looked up purely as the pooled scheme at lowering
/// time too — `mcrl2_lowering`'s builtin-name arm builds the concrete
/// `DataFunctionSymbol` directly from a use site's already-resolved sort, with
/// no matching `map` declaration required anywhere in the generated system
/// content — so a monomorphic `map ==: List(Nat) # List(Nat) -> Bool;` per
/// instantiated sort would be pure, unbounded bloat on `system` for no
/// consumer.
pub(crate) fn comparison_operator_equations(
    sources: &mut SourceMap,
    sort: &SortExpression,
) -> (UntypedDataSpecification, (TemplateId, Vec<SortExpression>)) {
    let template = register_bare_template(
        sources,
        "<builtin>/schemes/comparison.mcrl2",
        BUILTIN_SCHEME_TEMPLATE_TEXT,
        &BUILTIN_SCHEME_TEMPLATE,
    );
    let mut generated = replace_sort(&template, "S", sort);
    generated.map_declarations.clear();
    (generated, (TemplateId::Comparison, vec![sort.clone()]))
}

/// Builds the system-defined part of a specification: the Appendix-B
/// definitions (constructors, mappings and equations).
///
/// The five basic sorts are always included. fixpoint below
/// discovers by re-scanning each generated specification.
///
/// The comparison pass also feeds [basic_sort_dependencies] to its own
/// `expand_sorts` fixpoint as `extra_dependencies`: a numeric basic sort's own
/// Appendix-B template uses `if`/`==` on another basic sort purely as an
/// implementation detail — `nat.mcrl2`'s `pred` needs `if` on `Pos`,
/// `int.mcrl2` is defined in terms of `Pos`/`Nat`, and so on — whether or not
/// the user's specification ever mentions the other sort itself. Left alone,
/// such an instantiation would only be generated by coincidence, when that
/// other sort also happens to appear (textually or through inference) in the
/// user's own specification. Discovering this by rescanning each sort's own
/// template (gated on that sort already being in the worklist) rather than a
/// blanket scan of `basics` as a whole is what keeps this lazy: `Real`'s own
/// `min`/`max` use `if` on `Real` too, but that self-reference only fires once
/// `Real` is already being processed, so an unused `Real` still gets no
/// comparison equations of its own — see
/// `test_unused_basic_sort_has_no_comparison_equations`.
///
/// A function sort `D_0 # ... # D_{n-1} -> T` contributes the function-update
/// operators for its declared arity — the bundled single-argument template when
/// `n == 1`, otherwise [standard_sort] generalizes it to the flattened domain.
/// Structured-sort equations are generated separately from the desugared
/// declarations and merged in by `DataSpecification::from_untyped`.
///
/// The comparison-operator pass runs independently, over its own worklist and
/// `seen` set: unlike containers/functions, `==`/`<`/`if` apply uniformly to
/// any sort, so a sort can legitimately need both a container instantiation
/// and a comparison instantiation, and the two passes must not block each
/// other.
///
/// The result is deliberately left unresolved: it uses the built-in `Simple`
/// sorts and the Appendix-B operator names, and is not re-checked against the
/// user-oriented well-typedness rules.
///
/// `basics` is the
/// [`basic_sort_data_specification`](crate::basic_sort_data_specification),
/// passed in because the caller also needs it separately for the system
/// signature.
///
/// Returns the merged specification alongside the [TemplateInstantiation]s its
/// content was generated in.
pub(crate) fn build_system_defined_specification(
    sources: &mut SourceMap,
    spec: &UntypedDataSpecification,
    basics: UntypedDataSpecification,
    encoding: NumberEncoding,
) -> (UntypedDataSpecification, Vec<TemplateInstantiation>) {
    let mut result = basics;

    let mut container_worklist = Vec::new();
    // Seed from the user specification, including its function sorts.
    collect_system_sorts_in_spec(
        spec,
        &mut container_worklist,
        SortCollectionMode::ContainersAndFunctions,
    );
    let mut instantiations = merge_generated(
        sources,
        &mut result,
        container_worklist,
        &HashSet::new(),
        SortCollectionMode::ContainersOnly,
        |sources, sort| standard_sort(sources, sort, encoding),
        |_sort| Vec::new(),
    );

    let mut comparison_worklist = Vec::new();
    collect_system_sorts_in_spec(spec, &mut comparison_worklist, SortCollectionMode::Every);
    instantiations.extend(merge_generated(
        sources,
        &mut result,
        comparison_worklist,
        &HashSet::new(),
        SortCollectionMode::Every,
        comparison_operator_equations,
        |sort| basic_sort_dependencies(sort, spec, encoding),
    ));

    (result, instantiations)
}

/// The other basic sorts referenced as a free identifier somewhere in `sort`'s
/// own bundled Appendix-B template (in `encoding`) — `nat.mcrl2`'s `pred`
/// uses `if` on `Pos`, `int.mcrl2` is defined in terms of `Pos`/`Nat`,
/// `real.mcrl2`'s own constructor takes an `Int` argument, and so on —
/// discovered by rescanning the template text itself (via
/// [basic_sort_own_template]) instead of a hand-written dependency table, the
/// same way [expand_sorts] rescans a *generated* container's content for
/// further container dependencies. Used as `expand_sorts`'s
/// `extra_dependencies` for the comparison pass, so a comparison
/// instantiation `basics` itself needs internally gets generated even when
/// nothing in the user's specification otherwise reaches that sort — see
/// [build_system_defined_specification]'s doc comment. A non-basic `sort`
/// (a container, a user sort, …) has no such template and contributes
/// nothing.
///
/// Under [NumberEncoding::MachineWord], the `*64` templates additionally
/// reference the system-internal `@word` sort as a free identifier; resolved
/// against `spec`'s own `sort_declarations`, which must already carry
/// `@word`'s `SortId` by this point (true from
/// `build_system_defined_specification`/`extend_system_with_inferred_sorts`'s
/// own callers onward, once the system-internal sorts are folded in) —
/// pushing an unresolved [SortExpressionKind::Reference] instead would later
/// hit `sort_resolution.rs`'s "Names must have been resolved" panic, since
/// everything downstream of this point in the pipeline expects sorts to
/// already be `Simple`/`Resolved`. `@word` is deliberately the only bare
/// reference resolved this way: a template's own auxiliary sort (`nat.mcrl2`'s
/// `@NatPair`) is a free reference too, but it names an implementation detail
/// private to that one template, not a dependency on another basic sort's own
/// comparison instantiation — treating it as one would wrongly generate a
/// comparison instantiation for it, and is why this walks only `sort`'s own
/// cons/map declaration *leaves* rather than reusing
/// [collect_system_sorts_in_spec]'s general-purpose `Every` mode: that mode
/// pushes a whole `map`/`cons` signature's `Function`/`Complex` sort as one
/// node (it is written for already-resolved content, where that is safe), and
/// on a raw, unlinked template like `nat.mcrl2`'s own `@divmod: Pos # Pos ->
/// @NatPair`, the unresolved `@NatPair` inside that node would ride along
/// into the worklist untouched.
fn basic_sort_dependencies(
    sort: &SortExpression,
    spec: &UntypedDataSpecification,
    encoding: NumberEncoding,
) -> Vec<SortExpression> {
    let SortExpressionKind::Simple(basic) = &sort.node else {
        return Vec::new();
    };
    let template = basic_sort_own_template(*basic, encoding);

    let mut dependencies = Vec::new();
    let mut visit_sort = |sort: &SortExpression| {
        sort.visit::<(), _>(|expr| {
            match &expr.node {
                SortExpressionKind::Simple(_) => dependencies.push(expr.clone()),
                SortExpressionKind::Reference(name) if name == "@word" => {
                    if let Some(id) = spec
                        .sort_declarations
                        .iter()
                        .find(|decl| decl.identifier == "@word")
                        .and_then(|decl| decl.id)
                    {
                        dependencies.push(SortExpressionKind::Resolved(name.clone(), id).into());
                    }
                }
                _ => {}
            }
            ControlFlow::Continue(())
        });
    };
    for constructor in &template.constructor_declarations {
        visit_sort(&constructor.sort);
    }
    for map in &template.map_declarations {
        visit_sort(&map.sort);
    }

    dependencies
}

/// Drains `worklist` to a fixpoint: for every sort popped that has not already
/// been `seen`, generates its Appendix-B specification (plus whatever side
/// data `generate` wants carried through — [merge_generated] uses this for a
/// [TemplateInstantiation]'s template id and substitution; a caller with
/// nothing to carry uses `T = ()`) via `generate` and passes both to
/// `on_generated`, then re-scans the generated content (in `scan_mode`) for
/// further sorts it in turn depends on (a container is defined in terms of
/// other containers, e.g. `Set(S)` needs `FSet(S)`) and pushes those too.
///
/// `scan_mode` should be [SortCollectionMode::ContainersOnly] when `generate`
/// produces container content: function sorts are not re-collected from
/// generated container content (only from the initial `worklist`), since the
/// function-update operators introduce ever-larger function sorts
/// (`@is_not_an_update: (S -> T) -> Bool`), which would not terminate here.
/// Comparison-operator content has no such concern — a generated
/// instantiation only ever mentions the sort itself and `Bool` — so
/// [SortCollectionMode::Every] is safe there.
///
/// `extra_dependencies` additionally pushes whatever other sorts the popped
/// `sort` itself is known to depend on outside of `generated` — used by the
/// comparison pass to pull in a basic sort's own cross-references (see
/// [basic_sort_dependencies]); a container/function-update pass has none, so
/// it passes a closure that always returns empty. A self-reference (`Real`'s
/// own `min`/`max` use `if` on `Real`) is a no-op, exactly like a container
/// referencing itself: `seen` already dedupes it.
fn expand_sorts<T>(
    sources: &mut SourceMap,
    mut worklist: Vec<SortExpression>,
    seen: &mut HashSet<SortExpression>,
    scan_mode: SortCollectionMode,
    mut generate: impl FnMut(&mut SourceMap, &SortExpression) -> (UntypedDataSpecification, T),
    mut extra_dependencies: impl FnMut(&SortExpression) -> Vec<SortExpression>,
    mut on_generated: impl FnMut(UntypedDataSpecification, T),
) {
    while let Some(sort) = worklist.pop() {
        if !seen.insert(sort.clone()) {
            continue;
        }

        let (generated, extra) = generate(sources, &sort);
        collect_system_sorts_in_spec(&generated, &mut worklist, scan_mode);
        worklist.extend(extra_dependencies(&sort));
        on_generated(generated, extra);
    }
}

/// Extends `system` with the Appendix-B declarations of every container sort
/// and comparison-operator instantiation that is discovered only through
/// Phase-3 inference rather than appearing in the textual declarations: the
/// element sort of a `List`/`Set`/`Bag` enumeration literal (`[1, 2]`,
/// `{1, 2}`, `{1: 2}`), or of a bare numeral, is not written down anywhere —
/// it is entirely a product of its elements' inferred sorts (see
/// [collect_system_sorts_in_expr]'s doc comment) — so
/// [build_system_defined_specification]'s syntactic scan misses it whenever
/// the same sort does not also occur, spelled out, elsewhere in the
/// specification.
///
/// `ctx` must be the context [`crate::typecheck_equations`] populated: every sort
/// reachable from a successfully typed equation's per-node sorts is a
/// candidate. Which of those `system` already covers is not recorded anywhere
/// (containers are structural, not named, so `system` carries no direct list
/// of them), so the syntactic scan is replayed here to reconstruct that set
/// before diffing against it — once for containers/functions, once
/// independently for comparisons, mirroring
/// [build_system_defined_specification]'s own two independent passes,
/// [basic_sort_dependencies] included: a numeral literal's own sort (`Pos`,
/// almost always) is exactly the kind of inference-only sort this function
/// exists to catch, so this is the usual place the `Nat`/`@word` dependency
/// actually gets discovered from, not `build_system_defined_specification`'s
/// own (purely textual) scan.
///
/// Returns a new specification plus the [TemplateInstantiation]s of the newly
/// added content; `system` itself is left untouched, so calling this repeatedly (as
/// [crate::DataSpecification::lower_data_specification] may be) keeps
/// producing the same result from the same inputs.
pub(crate) fn extend_system_with_inferred_sorts(
    sources: &mut SourceMap,
    ctx: &TypeCheckContext,
    spec: &UntypedDataSpecification,
    system: &UntypedDataSpecification,
    encoding: NumberEncoding,
) -> (UntypedDataSpecification, Vec<TemplateInstantiation>) {
    let mut result = system.clone();

    // Reconstruct the set of container sorts `system` already covers.
    let mut container_seen: HashSet<SortExpression> = HashSet::new();
    let mut container_covered = Vec::new();
    collect_system_sorts_in_spec(spec, &mut container_covered, SortCollectionMode::ContainersAndFunctions);
    expand_sorts(
        sources,
        container_covered,
        &mut container_seen,
        SortCollectionMode::ContainersOnly,
        |sources, sort| (standard_sort(sources, sort, encoding).0, ()),
        |_sort| Vec::new(),
        |_, ()| {},
    );

    // Every container *or function* sort that shows up as the inferred sort
    // of some expression node in a well-typed equation, not already covered
    // above -- a function sort is included here too (not just containers)
    // because a lambda's own result sort is never spelled out by any
    // `map`/`cons` declaration or function-sort binder, yet function-update
    // syntax (`expr[key -> value]`) accepts any function-sorted `expr`, not
    // just a named mapping (`system_check.rs`'s `FunctionUpdate` arm recurses
    // into an arbitrary sub-expression with no such restriction). `standard_sort`
    // (the generator below) already handles both container and function
    // sorts uniformly, and `container_seen` already contains any function
    // sort reachable from a declared signature (from the
    // `ContainersAndFunctions` collection above), so this only adds the ones
    // that never appear in any declaration at all.
    //
    // A node present in `typing.names` is excluded: it is an `Id`/`Resolved`
    // reference, so its own sort is either a declared `map`/`cons`/`var`
    // overload (`NameTarget::Op`/`Variable`, already reachable from a
    // declaration and hence already covered above) or a builtin comparison's
    // per-use-site operator type (`NameTarget::Builtin`, e.g. `==`'s own
    // `(S # S) -> Bool`) -- the latter is never itself a function *value*
    // that function-update syntax could apply to, only the callee position of
    // an `Application`, so including it would generate a batch of
    // `@func_update` machinery nothing in the specification can ever produce
    // a stuck term without (and can name an arity `function_update_arities`
    // never pre-checked, panicking during instantiation).
    let mut container_worklist = Vec::new();
    for typing in ctx.equation_typing.values().filter_map(|typing| typing.as_ref().ok()) {
        for (index, &id) in typing.sorts.iter().enumerate() {
            let node = ExprId::new(index);
            let wanted = matches!(ctx.sorts.get(id), ResolvedSort::Container { .. })
                || (matches!(ctx.sorts.get(id), ResolvedSort::Function { .. }) && !typing.names.contains_key(&node));
            if wanted && let Some(sort) = resolved_sort_to_syntax(ctx, spec, id) {
                container_worklist.push(sort);
            }
        }
    }
    // `equation_typing` is a HashMap, so its iteration order (hence the push
    // order above) varies between runs; sort so `merge_generated` below sees
    // a fixed order and the generated equations end up in the same order
    // every time.
    container_worklist.sort();

    let mut instantiations = merge_generated(
        sources,
        &mut result,
        container_worklist,
        &container_seen,
        SortCollectionMode::ContainersOnly,
        |sources, sort| standard_sort(sources, sort, encoding),
        |_sort| Vec::new(),
    );

    // The comparison-operator counterpart: reconstruct the set of sorts
    // `system` already covers for comparisons (every sort, not just
    // containers/functions)...
    let mut comparison_seen: HashSet<SortExpression> = HashSet::new();
    let mut comparison_covered = Vec::new();
    collect_system_sorts_in_spec(spec, &mut comparison_covered, SortCollectionMode::Every);
    expand_sorts(
        sources,
        comparison_covered,
        &mut comparison_seen,
        SortCollectionMode::Every,
        |sources, sort| (comparison_operator_equations(sources, sort).0, ()),
        |sort| basic_sort_dependencies(sort, spec, encoding),
        |_, ()| {},
    );

    // ...then every sort that shows up as the inferred sort of some
    // expression node in a well-typed equation — `resolved_sort_to_syntax`
    // already returns `None` for the two `ResolvedSort` variants that never
    // denote a comparable data sort (`Var`, `Unit`), so no extra filter is
    // needed here beyond that.
    let mut comparison_worklist = Vec::new();
    for typing in ctx.equation_typing.values().filter_map(|typing| typing.as_ref().ok()) {
        for &id in &typing.sorts {
            if let Some(sort) = resolved_sort_to_syntax(ctx, spec, id) {
                comparison_worklist.push(sort);
            }
        }
    }
    comparison_worklist.sort();

    instantiations.extend(merge_generated(
        sources,
        &mut result,
        comparison_worklist,
        &comparison_seen,
        SortCollectionMode::Every,
        comparison_operator_equations,
        |sort| basic_sort_dependencies(sort, spec, encoding),
    ));

    // The freshly generated content still carries the raw `Binary`/`Unary`/
    // `List` nodes the templates are written with (mirroring what
    // `DataSpecification::from_untyped` does for the syntactically-collected
    // part); already-lowered content passes through unchanged since lowering
    // is idempotent.
    lower_data_expressions(&mut result);
    (result, instantiations)
}

/// Converts an inferred sort back into the `merc_syntax` sort-expression form
/// the Appendix-B templates are written in — the mirror of
/// `mcrl2_lowering::lower_sort`, but targeting the syntax tree rather than the
/// aterm schema, since [standard_sort] substitutes into syntax-tree templates.
/// Returns `None` for [ResolvedSort::Unit] (never a data sort) or a
/// [ResolvedSort::Def] whose declaration cannot be named (out of range of
/// `spec`, which does not happen for a sort that inference actually produced).
fn resolved_sort_to_syntax(
    ctx: &TypeCheckContext,
    spec: &UntypedDataSpecification,
    id: ResolvedSortId,
) -> Option<SortExpression> {
    match ctx.sorts.get(id) {
        // Never a data sort, like `Unit`: a bound type variable is always
        // instantiated to a fresh unification variable before Phase-3
        // solving produces a node's final ResolvedSortId, so this case does
        // not happen for a sort inference actually produced either.
        ResolvedSort::TypeVar(_) => None,
        ResolvedSort::Unit => None,
        ResolvedSort::Primitive(sort) => Some(SortExpressionKind::Simple(*sort).into()),
        ResolvedSort::Container { op, subsort } => {
            let sub = resolved_sort_to_syntax(ctx, spec, *subsort)?;
            Some(SortExpressionKind::Complex(*op, Box::new(sub)).into())
        }
        ResolvedSort::Function { domain, range } => {
            let domain = domain
                .iter()
                .map(|&sort| resolved_sort_to_syntax(ctx, spec, sort))
                .collect::<Option<Vec<_>>>()?;
            let range = resolved_sort_to_syntax(ctx, spec, *range)?;
            Some(
                SortExpressionKind::FlattenedFunction {
                    domain,
                    range: Box::new(range),
                }
                .into(),
            )
        }
        ResolvedSort::Def(def) => {
            let name = ctx.sort_name(spec, *def)?;
            Some(SortExpressionKind::Resolved(name.to_string(), *def).into())
        }
    }
}

/// Collects every container sort — every simple/resolved (basic or
/// user-declared) sort too, in [SortCollectionMode::Every] — and, unless
/// [SortCollectionMode::ContainersOnly], every single-argument function sort,
/// occurring in the specification into `out`, including the sorts on binders
/// inside the equation expressions.
pub(crate) fn collect_system_sorts_in_spec(
    spec: &UntypedDataSpecification,
    out: &mut Vec<SortExpression>,
    mode: SortCollectionMode,
) {
    for declaration in &spec.sort_declarations {
        if let Some(expr) = &declaration.expr {
            collect_system_sorts(expr, out, mode);
        }
    }

    for constructor in &spec.constructor_declarations {
        collect_system_sorts(&constructor.sort, out, mode);
    }

    for map in &spec.map_declarations {
        collect_system_sorts(&map.sort, out, mode);
    }

    for equation in &spec.equation_declarations {
        collect_system_sorts_in_equation(equation, out, mode);
    }
}

/// Collects the system-defined sorts occurring in a single `var ... eqn ...`
/// block: its declared variable sorts and its equations' conditions, left- and
/// right-hand sides (including binder sorts inside those expressions).
fn collect_system_sorts_in_equation(equation: &EqnSpec, out: &mut Vec<SortExpression>, mode: SortCollectionMode) {
    for variable in &equation.variables {
        collect_system_sorts(&variable.sort, out, mode);
    }

    for eqn in &equation.equations {
        if let Some(condition) = &eqn.condition {
            collect_system_sorts_in_expr(condition, out, mode);
        }

        collect_system_sorts_in_expr(&eqn.lhs, out, mode);
        collect_system_sorts_in_expr(&eqn.rhs, out, mode);
    }
}

/// Collects the system-defined sorts mentioned syntactically inside a data
/// expression: the sorts on binders, and around a set/bag comprehension's
/// element sort also `Set(S)` and `Bag(S)` — the comprehension denotes one of
/// the two, which reading applies is only decided by sort inference, so the
/// operators of both are provided. The element sorts of enumeration literals
/// (`{1, 2}`) are not syntactically apparent and are not collected.
///
/// Binder sorts that are not valid variable sorts (see
/// [is_supported_binder_sort]) are skipped: inference rejects the constructs
/// that bind them, so their operators are never looked up.
fn collect_system_sorts_in_expr(expr: &DataExpr, out: &mut Vec<SortExpression>, mode: SortCollectionMode) {
    expr.visit::<(), _>(|expr| {
        match &expr.node {
            DataExprKind::SetBagComp { variable, predicate: _ } => {
                if is_supported_binder_sort(&variable.sort) {
                    collect_system_sorts(&variable.sort, out, mode);
                    out.push(SortExpressionKind::Complex(ComplexSort::Set, Box::new(variable.sort.clone())).into());
                    out.push(SortExpressionKind::Complex(ComplexSort::Bag, Box::new(variable.sort.clone())).into());
                }
            }
            DataExprKind::Lambda { variables, body: _ }
            | DataExprKind::Quantifier {
                op: _,
                variables,
                body: _,
            } => {
                for variable in variables {
                    if is_supported_binder_sort(&variable.sort) {
                        collect_system_sorts(&variable.sort, out, mode);
                    }
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    });
}

/// Collects the system-defined sorts in a single sort expression, recursing
/// through element, function, product and structured sorts.
///
/// Container sorts are always collected. Function sorts of any arity are
/// collected unless [SortCollectionMode::ContainersOnly] — see the call in
/// [`build_system_defined_specification`] for why generated container content
/// is re-scanned without them. A single-argument domain is converted to the
/// nested `Function` form [`standard_sort`]'s `Function` branch expects; a
/// multi-argument domain is passed through as `FlattenedFunction`, which
/// `standard_sort`'s `FlattenedFunction` branch consumes directly. In
/// [SortCollectionMode::Every], every `Simple`/`Resolved` leaf sort is
/// collected too — `sort.visit` already recurses into every child regardless
/// of whether the current node was pushed, so a compound sort like
/// `List(Nat)` yields both itself and `Nat` with no extra recursion needed
/// here.
fn collect_system_sorts(sort: &SortExpression, out: &mut Vec<SortExpression>, mode: SortCollectionMode) {
    sort.visit::<(), _>(|expr| {
        match &expr.node {
            SortExpressionKind::Complex(_, _) => out.push(expr.clone()),
            // A user specification carries flattened function sorts; the
            // generated Appendix-B specifications carry the un-flattened
            // `Function` form.
            SortExpressionKind::Function { domain, .. } => {
                if mode != SortCollectionMode::ContainersOnly
                    && !matches!(domain.node, SortExpressionKind::Product { .. })
                {
                    out.push(expr.clone());
                }
            }
            SortExpressionKind::FlattenedFunction { domain, range } if mode != SortCollectionMode::ContainersOnly => {
                if let [single] = domain.as_slice() {
                    out.push(
                        SortExpressionKind::Function {
                            domain: Box::new(single.clone()),
                            range: range.clone(),
                        }
                        .into(),
                    );
                } else {
                    out.push(expr.clone());
                }
            }
            SortExpressionKind::Simple(_) | SortExpressionKind::Resolved(_, _) if mode == SortCollectionMode::Every => {
                out.push(expr.clone());
            }
            _ => {}
        }
        ControlFlow::Continue(())
    });
}

/// Specializes a template's own proven [`EquationTyping`] into the typing of
/// one concrete instantiation, substituting `substitution[i]` for `vars[i]`
/// throughout every sort the typing recorded.
pub(crate) fn specialize_template_typing(
    ctx: &mut TypeCheckContext,
    typing: &EquationTyping,
    vars: &[TypeVarId],
    substitution: &[ResolvedSortId],
) -> EquationTyping {
    debug_assert_eq!(
        vars.len(),
        substitution.len(),
        "one concrete sort per template type variable"
    );

    let substitute = |ctx: &mut TypeCheckContext, sort: ResolvedSortId| {
        vars.iter()
            .zip(substitution)
            .fold(sort, |sort, (&var, &with)| ctx.sorts.substitute_var(sort, var, with))
    };

    let sorts = typing.sorts.iter().map(|&sort| substitute(ctx, sort)).collect();
    let names = typing
        .names
        .iter()
        .map(|(&id, target)| {
            let target = match *target {
                NameTarget::Op { sort } => NameTarget::Op {
                    sort: substitute(ctx, sort),
                },
                other => other,
            };
            (id, target)
        })
        .collect();

    EquationTyping {
        sorts,
        spans: Vec::new(),
        names,
        identifier_names: HashMap::new(),
        declarations: HashMap::new(),
        node_ids: HashMap::new(),
    }
}

/// Populates `ctx.system_equation_typing` for every equation of `generated`.
///
/// `generated` is assembled *entirely* from template instantiations:
/// `build_system_defined_specification`/`extend_system_with_inferred_sorts` never append an
/// equation block to it except through `merge_generated`, which records one
/// [`TemplateInstantiation`] for every block it appends. So every block has a proven, rigid
/// template typing (`ctx.template_typings`, populated once up front by
/// `check_container_templates`/`check_comparison_template`/
/// `check_function_update_template` during `DataSpecification::from_untyped_with`)
/// to specialize from, and this function never calls `infer_equation` — only
/// [`specialize_template_typing`]'s substitution. The `debug_assert_eq!` below pins that coverage
/// down as a checked property rather than a comment.
pub(crate) fn instantiate_system_equations(
    ctx: &mut TypeCheckContext,
    user_spec: &UntypedDataSpecification,
    generated: &UntypedDataSpecification,
    instantiations: &[TemplateInstantiation],
) {
    debug_assert_eq!(
        instantiations
            .iter()
            .map(|instantiation| instantiation.equation_range.len())
            .sum::<usize>(),
        generated.equation_declarations.len(),
        "every generated equation block must come from exactly one template instantiation"
    );

    for instantiation in instantiations {
        let substitution: Vec<ResolvedSortId> = instantiation
            .substitution
            .iter()
            // Drawn from the user's own already-resolved sort tree, so `resolve_sort` resolves
            // every entry infallibly.
            .map(|sort| resolve_sort(ctx, user_spec, sort))
            .collect();

        for (local_index, block_index) in instantiation.equation_range.clone().enumerate() {
            instantiate_equation_block(ctx, generated, instantiation, &substitution, block_index, local_index);
        }
    }
}

/// Specializes one `generated.equation_declarations[block_index]` block, the local `local_index`-th
/// one `instantiation`'s own template contributes.
fn instantiate_equation_block(
    ctx: &mut TypeCheckContext,
    generated: &UntypedDataSpecification,
    instantiation: &TemplateInstantiation,
    substitution: &[ResolvedSortId],
    block_index: usize,
    local_index: usize,
) {
    let eqn_spec = &generated.equation_declarations[block_index];
    let eqn_spec_id = eqn_spec
        .id
        .expect("assign_declaration_ids ran on the generated content before instantiate_system_equations");

    let check = ctx.template_typings.get(&instantiation.template).unwrap_or_else(|| {
        panic!(
            "template '{}' was not checked before instantiation — every template is proven once, \
             up front, by `DataSpecification::from_untyped_with`",
            instantiation.template
        )
    });
    let type_vars = check.type_vars.clone();
    let block_typings = check.typings[local_index].clone();

    for (equation, template_typing) in eqn_spec.equations.iter().zip(&block_typings) {
        let equation_id = equation
            .id
            .expect("assign_declaration_ids ran on the generated content before instantiate_system_equations");
        let specialized = specialize_template_typing(ctx, template_typing, &type_vars, substitution);
        ctx.system_equation_typing
            .insert((eqn_spec_id, equation_id), Ok(Arc::new(specialized)));
    }
}

/// Replaces the given `type_var`-declared identifier by the given sort
/// expression in the given data specification.
///
/// # Details
///
/// This function can be used to instantiate polymorphic types, for example,
/// replacing `spec`'s bound type variable `S` (declared by `spec`'s own
/// `type_var S;` block) by `Nat` to get a specification for `List(Nat)` out
/// of the `List(S)` template. The substitution covers every sort in the
/// specification, including the binder sorts inside equations (`forall c:S.`
/// in the set/bag templates).
///
/// `identifier` is looked up in `spec.type_var_declarations` to find the
/// [TypeVarId] name resolution already assigned it.
fn replace_sort(spec: &UntypedDataSpecification, identifier: &str, sort: &SortExpression) -> UntypedDataSpecification {
    let mut result = spec.clone();

    let type_var_id = spec
        .type_var_declarations
        .iter()
        .find(|decl| decl.identifier == identifier)
        .and_then(|decl| decl.id)
        .unwrap_or_else(|| panic!("template has no resolved `type_var {identifier}` declaration"));

    apply_sorts_in_spec(&mut result, |expr| -> Result<_, Infallible> {
        Ok(replace_type_var(expr, type_var_id, sort))
    })
    .expect("substitution never fails");

    // `identifier` is now fully substituted away; drop its declaration so that.
    result
        .type_var_declarations
        .retain(|decl| decl.identifier != identifier);

    result
}

/// Replaces every [SortExpressionKind::ResolvedTypeVar] node naming `type_var_id` in `sort` by
/// `result_sort`. See [replace_sort].
fn replace_type_var(sort: &SortExpression, type_var_id: TypeVarId, result_sort: &SortExpression) -> SortExpression {
    sort.clone()
        .apply(|expr| -> Result<Option<SortExpression>, Infallible> {
            if let SortExpressionKind::ResolvedTypeVar(id) = &expr.node
                && *id == type_var_id
            {
                return Ok(Some(result_sort.clone()));
            }

            Ok(None)
        })
        .unwrap()
}

#[cfg(test)]
mod tests {
    use merc_syntax::ComplexSort;
    use merc_syntax::SortExpressionKind;
    use merc_syntax::SourceMap;
    use merc_syntax::UntypedDataSpecification;

    use super::SortCollectionMode;
    use super::build_system_defined_specification;
    use super::collect_system_sorts_in_spec;
    use crate::DataSpecification;
    use crate::NumberEncoding;
    use crate::basic_sort_data_specification;

    /// The distinct container constructors that occur in a specification.
    fn container_ops(spec: &UntypedDataSpecification) -> Vec<ComplexSort> {
        let mut sorts = Vec::new();
        collect_system_sorts_in_spec(spec, &mut sorts, SortCollectionMode::ContainersAndFunctions);
        let mut ops: Vec<ComplexSort> = sorts
            .into_iter()
            .filter_map(|sort| match sort.node {
                SortExpressionKind::Complex(op, _) => Some(op),
                _ => None,
            })
            .collect();
        ops.sort();
        ops.dedup();
        ops
    }

    fn system_spec(text: &str) -> UntypedDataSpecification {
        let mut sources = SourceMap::new();
        let basics = basic_sort_data_specification(&mut sources, NumberEncoding::Binary);
        build_system_defined_specification(
            &mut sources,
            &UntypedDataSpecification::parse(text).unwrap(),
            basics,
            NumberEncoding::Binary,
        )
        .0
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_basic_sorts_are_always_present() {
        let spec = system_spec("map f: Bool;");
        for basic in ["Bool", "Pos", "Nat", "Int", "Real"] {
            assert!(
                spec.sort_declarations.iter().any(|decl| decl.identifier == basic),
                "the basic sort {basic} should always be included"
            );
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_set_pulls_in_finite_set() {
        // A `Set(S)` is defined in terms of `FSet(S)`, so both must be present.
        let ops = container_ops(&system_spec("map f: Set(Nat);"));
        assert!(ops.contains(&ComplexSort::Set));
        assert!(ops.contains(&ComplexSort::FSet));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_comprehension_contributes_set_and_bag() {
        // A comprehension may denote a set or a bag; the equations of both are
        // provided for its element sort even though no declaration mentions a
        // container.
        let spec = UntypedDataSpecification::parse("map b: Bool; eqn b = 1 in { n: Pos | n < 3 };").unwrap();
        let ops = container_ops(&spec);
        for op in [ComplexSort::Set, ComplexSort::Bag] {
            assert!(ops.contains(&op), "a comprehension should contribute {op:?}");
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_quantifier_binder_sort_is_collected() {
        // The `List(Nat)` mentioned only on the quantifier binder still gets
        // its Appendix-B equations.
        let spec = UntypedDataSpecification::parse("map b: Bool; eqn b = forall l: List(Nat). l == [];").unwrap();
        assert!(container_ops(&spec).contains(&ComplexSort::List));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_bag_pulls_in_all_related_containers() {
        // A `Bag(S)` transitively needs `FBag(S)`, `FSet(S)` and `Set(S)`.
        let ops = container_ops(&system_spec("map f: Bag(Nat);"));
        for op in [ComplexSort::Bag, ComplexSort::FBag, ComplexSort::FSet, ComplexSort::Set] {
            assert!(ops.contains(&op), "using Bag should pull in {op:?}");
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_nested_container_element_is_included() {
        // `List(Set(Nat))` needs both the list and the (transitive) set defs.
        let ops = container_ops(&system_spec("map f: List(Set(Nat));"));
        assert!(ops.contains(&ComplexSort::List));
        assert!(ops.contains(&ComplexSort::Set));
        assert!(ops.contains(&ComplexSort::FSet));
    }

    /// Whether the *lowered* spec of `text` declares the function-update
    /// operators, checked through the full `from_untyped`/`lower_data_specification`
    /// path: that's where a container/function-update instantiation is
    /// generated at all — `system_defined_specification()` itself no longer
    /// carries one.
    fn has_function_update(text: &str) -> bool {
        let spec = DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()).unwrap();
        spec.lower_data_specification()
            .mappings()
            .iter()
            .any(|map| map.name().value().contains("func_update"))
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_single_argument_function_gets_update_operators() {
        assert!(has_function_update("map f: Nat -> Bool;"));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_multi_argument_function_gets_update_operators() {
        // `Nat # Bool -> Nat` has a product domain; `standard_sort` generalizes
        // the Appendix-B template to it instead of deferring it.
        assert!(has_function_update("map f: Nat # Bool -> Nat;"));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_function_over_containers_terminates() {
        // Regression: re-scanning generated function-update specs for further
        // function sorts diverged, because `@is_not_an_update: (S -> T) -> Bool`
        // is itself a single-argument function, growing the sort without bound.
        assert!(has_function_update("map f: List(Nat) -> List(Nat);"));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_multi_argument_function_over_containers_terminates() {
        // The same regression as above, but seeded from a multi-argument
        // function so the fixpoint also terminates when it re-scans a
        // generated multi-argument `@func_update`/`@is_not_an_update`/
        // `@if_always_else` specification.
        assert!(has_function_update("map f: List(Nat) # Bool -> List(Nat);"));
    }

    /// Whether `spec` includes the generic `if(true, x, y) = x;` reduction
    /// instantiated for `sort_name` — a `var`/`eqn` block whose declared
    /// variable has sort `sort_name` and whose equations reduce a generic
    /// `if`.
    fn spec_has_comparison_equations_for(spec: &UntypedDataSpecification, sort_name: &str) -> bool {
        spec.equation_declarations.iter().any(|eqn_spec| {
            eqn_spec.variables.iter().any(|var| var.sort.to_string() == sort_name)
                && eqn_spec.equations.iter().any(|eqn| eqn.lhs.to_string().contains("if("))
        })
    }

    /// As [spec_has_comparison_equations_for], but over the *lowered* spec,
    /// checked through the full `from_untyped`/`lower_data_specification` path
    /// (which flattens function sorts, desugars structs, drives Phase-3
    /// inference, and — since the monomorphization-to-lowering milestone — is
    /// also where a comparison-operator instantiation is generated at all).
    ///
    /// Compares by aterm equality rather than `Display`: `SortCons`'s own
    /// `Display` renders only the element sort (`"Nat"`, not `"List(Nat)"`) —
    /// a binary-aterm-format quirk unrelated to this milestone, since the
    /// container *kind* is a separate structural tag there, not part of a
    /// name — so a `sort_name` like `"List(Nat)"` is instead parsed and
    /// lowered through the same [`crate::lower_syntax_sort`] every other
    /// declaration sort goes through, and compared against that.
    fn has_comparison_equations_for(text: &str, sort_name: &str) -> bool {
        let spec = DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()).unwrap();
        let lowered = spec.lower_data_specification();

        let sort_spec = UntypedDataSpecification::parse(&format!("map q: {sort_name};")).unwrap();
        let expected_sort = crate::lower_syntax_sort(&sort_spec.map_declarations[0].sort);

        lowered.equations().iter().any(|eqn| {
            eqn.variables()
                .into_iter()
                .any(|var| var.sort().protect() == expected_sort)
                && eqn.lhs().to_string().contains("if(")
        })
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_container_sort_gets_comparison_equations_direct() {
        // As `test_container_sort_gets_comparison_equations`, but exercises
        // only the worklist/generation layer directly
        // (`build_system_defined_specification`), independent of the rest of
        // the type-checking pipeline.
        assert!(spec_has_comparison_equations_for(
            &system_spec("map f: List(Nat);"),
            "List(Nat)"
        ));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_unused_basic_sort_has_no_comparison_equations_direct() {
        // As `test_unused_basic_sort_has_no_comparison_equations`, checked
        // directly against `build_system_defined_specification`'s output.
        assert!(!spec_has_comparison_equations_for(&system_spec("map f: Bool;"), "Real"));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_container_sort_gets_comparison_equations() {
        // Closes a real gap: `if(b, xs, ys)` for `List(Nat)` used to
        // type-check (the polymorphic scheme accepts any sort) but had no
        // equation to rewrite it with, since `list.mcrl2` defines its own
        // structural `==`/`<` but never a generic `if`.
        assert!(has_comparison_equations_for("map f: List(Nat);", "List(Nat)"));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_struct_sort_gets_comparison_equations() {
        // A `struct` gets its own componentwise `==`/`<`/`<=` from
        // `structured_sort_equations`, but never `if` — that still has to
        // come from the generic scheme.
        assert!(has_comparison_equations_for("sort D = struct c1 | c2; map f: D;", "D"));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_unused_basic_sort_has_no_comparison_equations() {
        // The comparison-operator instantiation is lazy, like a container's:
        // a basic sort that never occurs in the specification gets no
        // comparison equations, even though its own sort/arithmetic
        // declarations are still unconditionally present.
        assert!(!has_comparison_equations_for("map f: Bool;", "Real"));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_sort_inferred_only_from_a_literal_gets_comparison_equations() {
        // The inference-driven pass (`extend_system_with_inferred_sorts`)
        // must catch a sort that is never spelled out anywhere in the
        // specification's own text — the comparison-operator counterpart of
        // the enumeration-literal container gap.
        assert!(has_comparison_equations_for("map f: Bool; eqn f = (1 == 1);", "Pos"));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_single_argument_function_uses_generalized_update_operators() {
        // A single-argument function sort takes the very same generation path
        // as a multi-argument one (arity 1 instead of arity > 1) — there is no
        // separate bundled template anymore.
        let mut sources = SourceMap::new();
        let checked = DataSpecification::from_untyped_with(
            UntypedDataSpecification::parse("map f: Nat -> Bool;").unwrap(),
            NumberEncoding::default(),
            &mut sources,
        )
        .unwrap();
        let sort = &checked.data_specification().map_declarations[0].sort;

        let (generated, _) = super::standard_sort(&mut sources, sort, NumberEncoding::Binary);
        let equations: Vec<String> = generated
            .equation_declarations
            .iter()
            .flat_map(|eqn_spec| &eqn_spec.equations)
            .map(|eqn| eqn.to_string())
            .collect();

        assert!(
            equations.iter().any(|eqn| eqn.contains("@func_update(@f, @x0, @v)")),
            "expected @func_update applied with the single index argument: {equations:#?}"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_standard_sort_substitutes_type_var_for_element_sort() {
        // `List(Nat)`'s `[]` constructor should end up with sort `List(Nat)` —
        // the type-var-based substitution must produce the same result the old
        // Reference-based one did — and no `type_var` declaration should be left
        // over in the instantiated copy.
        let mut sources = SourceMap::new();
        let checked = DataSpecification::from_untyped_with(
            UntypedDataSpecification::parse("map f: List(Nat);").unwrap(),
            NumberEncoding::default(),
            &mut sources,
        )
        .unwrap();
        let sort = &checked.data_specification().map_declarations[0].sort;

        let (generated, _) = super::standard_sort(&mut sources, sort, NumberEncoding::Binary);
        assert!(generated.type_var_declarations.is_empty(), "{generated}");

        let nil = generated
            .constructor_declarations
            .iter()
            .find(|cons| cons.identifier.node == "[]")
            .expect("List(Nat) should still declare `[]`");
        assert_eq!(nil.sort.to_string(), "List(Nat)");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_multi_argument_function_gets_generalized_update_operators() {
        // `from_untyped` flattens `Nat # Bool -> Nat` before generating the
        // system-defined specification, so `standard_sort` sees a
        // `FlattenedFunction` domain of length two and takes the
        // multi-argument branch instead of the bundled single-argument
        // template.
        let mut sources = SourceMap::new();
        let checked = DataSpecification::from_untyped_with(
            UntypedDataSpecification::parse("map f: Nat # Bool -> Nat;").unwrap(),
            NumberEncoding::default(),
            &mut sources,
        )
        .unwrap();
        let sort = &checked.data_specification().map_declarations[0].sort;
        let SortExpressionKind::FlattenedFunction { domain, .. } = &sort.node else {
            panic!("expected a flattened function sort: {sort}");
        };
        assert_eq!(domain.len(), 2);

        let (generated, _) = super::standard_sort(&mut sources, sort, NumberEncoding::Binary);
        assert!(
            generated
                .map_declarations
                .iter()
                .any(|map| map.identifier.node == "@func_update"),
            "the multi-argument function sort should still declare @func_update"
        );

        let equations: Vec<String> = generated
            .equation_declarations
            .iter()
            .flat_map(|eqn_spec| &eqn_spec.equations)
            .map(|eqn| eqn.to_string())
            .collect();

        // Both `f` and `@func_update` are applied with the full two-argument
        // index tuple, not the single index the bundled template uses.
        assert!(
            equations.iter().any(|eqn| eqn.contains("@f(@x0, @x1)")),
            "expected a two-argument application of f: {equations:#?}"
        );
        assert!(
            equations
                .iter()
                .any(|eqn| eqn.contains("@func_update(@f, @x0, @x1, @v)")),
            "expected @func_update applied with both index arguments: {equations:#?}"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_multi_argument_function_update_tolerates_user_overloads_of_its_bound_variable_names() {
        // The arity-2 function-update template's own equations bind `@f`, `@v`,
        // `@w`, `@x0`/`@x1`, `@y0`/`@y1` — checking them isolates the template's
        // scheme and merges it over the pooled signature (see
        // `check_function_update_template`), so an unrelated user mapping
        // literally named `f`, `v`, `x0`, ... would, without the `@`-prefix
        // reserved-name convention, shadow-collide with the template's own
        // bound variable of the same name and get reported as a spurious
        // ambiguity. No user declaration may start with `@`, ruling that out
        // by construction.
        let result = DataSpecification::from_untyped(
            UntypedDataSpecification::parse(
                "map f: Bool -> Bool; v: Bool -> Bool; w: Bool -> Bool; \
                 x0: Bool -> Bool; x1: Bool -> Bool; y0: Bool -> Bool; y1: Bool -> Bool; \
                 g: Nat # Bool -> Nat;",
            )
            .unwrap(),
        );
        if let Err(err) = result {
            panic!(
                "a user mapping sharing a name with the function-update template's own bound \
                 variables must not be reported as ambiguous: {err}"
            );
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_multi_argument_function_update_generalizes_to_higher_arities() {
        // The same construction must not be hard-coded to arity two.
        let mut sources = SourceMap::new();
        let checked = DataSpecification::from_untyped_with(
            UntypedDataSpecification::parse("map f: Nat # Bool # Nat -> Bool;").unwrap(),
            NumberEncoding::default(),
            &mut sources,
        )
        .unwrap();
        let sort = &checked.data_specification().map_declarations[0].sort;

        let (generated, _) = super::standard_sort(&mut sources, sort, NumberEncoding::Binary);
        let equations: Vec<String> = generated
            .equation_declarations
            .iter()
            .flat_map(|eqn_spec| &eqn_spec.equations)
            .map(|eqn| eqn.to_string())
            .collect();
        assert!(
            equations
                .iter()
                .any(|eqn| eqn.contains("@func_update(@f, @x0, @x1, @x2, @v)")),
            "expected @func_update applied with all three index arguments: {equations:#?}"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_standard_sort_substitutes_binder_sorts() {
        // The set template's `==` equation quantifies over the element sort
        // (`forall c:S.`); instantiation must substitute binder sorts like any
        // declaration sort, or the generated equation would reference the
        // undeclared `S`.
        let spec = UntypedDataSpecification::parse("map f: Set(Nat);").unwrap();
        let (generated, _) = super::standard_sort(
            &mut SourceMap::new(),
            &spec.map_declarations[0].sort,
            NumberEncoding::Binary,
        );

        let equations: Vec<String> = generated
            .equation_declarations
            .iter()
            .flat_map(|eqn_spec| &eqn_spec.equations)
            .map(|eqn| eqn.to_string())
            .collect();
        assert!(
            equations.iter().any(|eqn| eqn.contains("forall c: Nat")),
            "the quantifier's binder sort should be instantiated: {equations:#?}"
        );
    }
}
