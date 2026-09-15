use std::collections::HashMap;
use std::sync::Arc;

use merc_syntax::TypeVarId;
use merc_syntax::UntypedDataSpecification;

use crate::BUILTIN_SCHEME_TEMPLATE;
use crate::CONTAINER_TEMPLATES;
use crate::PolySortScheme;
use crate::ResolvedSortId;
use crate::Signature;
use crate::TypeCheckContext;
use crate::WellTypedError;
use crate::push_declarations;
use crate::push_overload;
use crate::resolve_sort;

/// Resolves the constructor and mapping declarations of the *basic-sort* part
/// of the system-defined specification onto the interned sort lattice, merging
/// them into `ctx.signature` (the same pooled signature the user's own
/// declarations resolve through — see `docs/typecheck.md`'s trusted-signature
/// milestone) — so a name like `succ`/`&&`/`@c0` is one more overload set in
/// the one table `gen_name` searches, not a second signature to fall back to.
///
/// `system` must be the *basic-sort* specification ([`basic_sort_data_specification`](crate::basic_sort_data_specification)),
/// not the full system-defined specification `build_system_defined_specification`
/// produces: the container operations are looked up polymorphically instead
/// (`ctx.signature.schemes`), because resolving their per-sort instantiations
/// here as well would misreport ambiguity (a name would have both a concrete
/// and a polymorphic candidate for the same sort).
///
/// Every `Reference` node of `system`'s own declarations must already be
/// resolved to `Resolved(name, SortId)` (see `DataSpecification::from_untyped_with`,
/// which folds `@NatPair`/`@word` into the shared `sort_declarations` table and
/// resolves `system` against it the same way it resolves `spec` itself) — so
/// `resolve_sort` is infallible here, the same call the user's own signature
/// resolves through.
///
/// Runs the same [`push_declarations`] well-typedness checks `build_signature` runs for the user's
/// own declarations, `trusted` (skipping only the basic-sort-constructor rule `@c0: Nat` and
/// friends legitimately break) — this is a real soundness check on the generated content, not a
/// user-only courtesy, and catches what `check_constructor_target` used to hand-check on its own
/// (no constructor for a function sort), plus disjointness and duplicate-constant-different-sort
/// checks that specification never ran on system content before.
///
/// Requires `build_signature` to have already populated `ctx.signature` with
/// the user's own declarations, so there is something to merge into.
///
/// Also stores the basics-only signature on `ctx.basics_signature` (not just the merged
/// `ctx.signature`), for `DataSpecification::from_untyped_with`'s own struct-equation signature
/// override, which must stay scoped to a struct's own names plus the basic-sort operators — never
/// the rest of the user's own signature, which could otherwise make an unrelated same-named user
/// declaration a spurious extra overload of a struct's own constructor/projection.
pub(crate) fn resolve_system_signature(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    system: &UntypedDataSpecification,
) -> Result<(), WellTypedError> {
    let mut signature = Signature::default();
    let mut constants: HashMap<String, ResolvedSortId> = HashMap::new();
    push_declarations(ctx, system, spec, true, &mut signature, &mut constants)?;

    for decl in &system.constructor_declarations {
        let id = resolve_sort(ctx, spec, &decl.sort);
        ctx.system_symbol_spans
            .insert((decl.identifier.node.clone(), id), decl.identifier.span.clone());
    }
    for decl in &system.map_declarations {
        let id = resolve_sort(ctx, spec, &decl.sort);
        ctx.system_symbol_spans
            .insert((decl.identifier.node.clone(), id), decl.identifier.span.clone());
    }

    let merged = merge_signatures(
        ctx.signature
            .as_deref()
            .expect("build_signature ran before resolve_system_signature"),
        &signature,
    );
    ctx.signature = Some(Arc::new(merged));
    ctx.basics_signature = Some(Arc::new(signature));
    Ok(())
}

/// Resolves the system-defined specification's declarations onto the interned
/// sort lattice and records each one's own declaration span
/// (`ctx.system_symbol_spans`, read back by `TypingInfo` for go-to-
/// definition).
pub(crate) fn resolve_system_signature_full(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    system: &UntypedDataSpecification,
) {
    for decl in &system.constructor_declarations {
        let id = resolve_sort(ctx, spec, &decl.sort);
        ctx.system_symbol_spans
            .insert((decl.identifier.node.clone(), id), decl.identifier.span.clone());
    }
    for decl in &system.map_declarations {
        let id = resolve_sort(ctx, spec, &decl.sort);
        ctx.system_symbol_spans
            .insert((decl.identifier.node.clone(), id), decl.identifier.span.clone());
    }
}

/// The subset of `signature` naming exactly `constructor_names` and
/// `mapping_names`, used to scope a struct's equations to its own symbols.
///
/// Two separate name sets, not one checked against both maps: a struct's
/// constructor can share a name with an unrelated struct's projection (`a` as
/// both a constant and a projection in `struct.mcrl2`), and only the
/// constructor/mapping distinction tells the two apart.
pub(crate) fn filter_signature(
    signature: &Signature,
    constructor_names: &std::collections::HashSet<String>,
    mapping_names: &std::collections::HashSet<String>,
) -> Signature {
    let mut filtered = Signature::default();
    for name in constructor_names {
        if let Some(overloads) = signature.constructors.get(name) {
            filtered.constructors.insert(name.clone(), overloads.clone());
        }
    }
    for name in mapping_names {
        if let Some(overloads) = signature.mappings.get(name) {
            filtered.mappings.insert(name.clone(), overloads.clone());
        }
    }
    filtered
}

/// The union of `a` and `b`'s overload sets, per name — ground overloads
/// deduplicated by id, scheme overloads simply concatenated (two schemes
/// never denote the same overload the way a ground redeclaration can).
pub(crate) fn merge_signatures(a: &Signature, b: &Signature) -> Signature {
    let mut merged = Signature {
        constructors: a.constructors.clone(),
        mappings: a.mappings.clone(),
        schemes: a.schemes.clone(),
    };
    for (name, overloads) in &b.constructors {
        let entry = merged.constructors.entry(name.clone()).or_default();
        for &id in overloads {
            push_overload(entry, id);
        }
    }
    for (name, overloads) in &b.mappings {
        let entry = merged.mappings.entry(name.clone()).or_default();
        for &id in overloads {
            push_overload(entry, id);
        }
    }
    for (name, schemes) in &b.schemes {
        merged
            .schemes
            .entry(name.clone())
            .or_default()
            .extend(schemes.iter().cloned());
    }
    merged
}

/// Builds one [PolySortScheme] per constructor/mapping declaration of each
/// `template` in `templates`, keyed by name, via [`resolve_sort`] against the
/// template's own (self-contained) spec — legal because every occurrence of
/// the template's own `type_var` block interns to the same [ResolvedSort::Var](crate::ResolvedSort::Var),
/// on the same footing as any other lattice element.
///
/// Safe to call with any of [CONTAINER_TEMPLATES]/[BUILTIN_SCHEME_TEMPLATE]:
/// none of them contains a `Resolved(_, SortId)` node or a nominal `sort X;`
/// declaration (only `type_var`, primitive, container and function sorts), so
/// there is no `SortId` to resolve and hence no risk of it being looked up
/// against the wrong spec's `sort_declarations`.
///
/// This is the one shared mechanism behind both `ctx.signature`'s `schemes`
/// (containers, function-update and the comparison/`if` builtins, for the
/// user-facing lookup — see `build_signature`) and
/// [`build_builtin_scheme_signature`]'s narrower table (the comparison/`if`
/// builtins only, for a struct-scoped system equation's own lookup).
pub(crate) fn build_polymorphic_schemes<'a>(
    ctx: &mut TypeCheckContext,
    templates: impl IntoIterator<Item = &'a UntypedDataSpecification>,
) -> HashMap<String, Vec<PolySortScheme>> {
    let mut schemes: HashMap<String, Vec<PolySortScheme>> = HashMap::new();
    for template in templates {
        let vars: Vec<TypeVarId> = template
            .type_var_declarations
            .iter()
            .filter_map(|decl| decl.id)
            .collect();
        for (identifier, sort) in template
            .constructor_declarations
            .iter()
            .map(|decl| (&decl.identifier, &decl.sort))
            .chain(
                template
                    .map_declarations
                    .iter()
                    .map(|decl| (&decl.identifier, &decl.sort)),
            )
        {
            let resolved = resolve_sort(ctx, template, sort);
            schemes
                .entry(identifier.node.clone())
                .or_default()
                .push(PolySortScheme {
                    vars: vars.clone(),
                    sort: resolved,
                });
        }
    }
    schemes
}

/// The narrow scheme table a struct-scoped system equation's own body is checked against: the
/// comparison operators and `if` only, built once and cached on `ctx`. Deliberately excludes the
/// container/function-update templates — reached only by a struct's own isolated equations
/// (`ctx.struct_signature_overrides`) or the plain basics equations
/// (`ctx.basics_signature`), neither of which ever calls a container operation, so admitting them
/// here polymorphically would only risk misreporting ambiguity against a struct override's own
/// real symbols for no benefit. A container/function-update instantiation's own equations are
/// specialized from their template's proven typing instead of reaching this role at all — see
/// [`crate::check_system_equations`]'s `instantiations` parameter.
pub(crate) fn build_builtin_scheme_signature(ctx: &mut TypeCheckContext) -> Arc<HashMap<String, Vec<PolySortScheme>>> {
    if ctx.builtin_scheme_signature.is_none() {
        let schemes = build_polymorphic_schemes(ctx, std::iter::once(&*BUILTIN_SCHEME_TEMPLATE));
        ctx.builtin_scheme_signature = Some(Arc::new(schemes));
    }
    Arc::clone(
        ctx.builtin_scheme_signature
            .as_ref()
            .expect("just computed above if it wasn't already"),
    )
}

/// The reserved names of every polymorphic built-in operator (containers,
/// function-update, comparisons/`if`) — a user `cons`/`map` declaration may
/// not redeclare any of them, regardless of its own sort. Derived directly
/// from the templates rather than from `ctx.signature`'s schemes, since this
/// check runs early in the pipeline, well before a `TypeCheckContext` (and so
/// a `Signature`) exists.
pub(crate) fn polymorphic_operator_names() -> impl Iterator<Item = &'static str> {
    CONTAINER_TEMPLATES
        .all()
        .into_iter()
        .flat_map(|template| {
            template
                .constructor_declarations
                .iter()
                .map(|decl| decl.identifier.as_str())
                .chain(template.map_declarations.iter().map(|decl| decl.identifier.as_str()))
        })
        .chain(crate::builtin_scheme_names())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use merc_syntax::ComplexSort;
    use merc_syntax::Sort;
    use merc_syntax::SortId;
    use merc_syntax::SourceMap;
    use merc_syntax::UntypedDataSpecification;

    use crate::DataSpecification;
    use crate::NumberEncoding;
    use crate::ResolvedSort;
    use crate::ResolvedSortId;
    use crate::Signature;
    use crate::TypeCheckContext;
    use crate::WellTypedError;
    use crate::basic_sort_data_specification;
    use crate::build_system_defined_specification;
    use crate::merge_signatures;
    use crate::resolve_system_signature;

    /// Type checks `text` and resolves the basic-sort system signature in a
    /// fresh context, as `DataSpecification::from_untyped` does.
    fn resolve(text: &str) -> (DataSpecification, TypeCheckContext) {
        let mut sources = SourceMap::new();
        let spec = DataSpecification::from_untyped_with(
            UntypedDataSpecification::parse(text).unwrap(),
            NumberEncoding::default(),
            &mut sources,
        )
        .unwrap();
        let mut ctx = TypeCheckContext::new();
        // `resolve_system_signature` merges into `ctx.signature`, so there must be one to merge
        // into, exactly as in the real pipeline.
        crate::build_signature(&mut ctx, spec.data_specification()).unwrap();
        // Mirrors `from_untyped_with`'s own resolution of `basics` against the
        // spec's shared `sort_declarations` table (which already carries
        // `@NatPair`/`@word`, folded in by that same pipeline run).
        let mut basics = basic_sort_data_specification(&mut sources, NumberEncoding::Binary);
        crate::apply_sorts_in_spec(&mut basics, |sort| crate::resolve_sort_id(sort, spec.sorts())).unwrap();
        resolve_system_signature(&mut ctx, spec.data_specification(), &basics).unwrap();
        (spec, ctx)
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_boolean_operators_are_resolved() {
        let (_, ctx) = resolve("map f: Bool;");
        let signature = ctx.signature.as_ref().unwrap();

        let bool_sort = ctx.sorts.primitive(Sort::Bool);
        let conjunction = ctx.sorts.get(signature.mappings["&&"][0]).clone();
        assert_eq!(
            conjunction,
            ResolvedSort::Function {
                domain: vec![bool_sort, bool_sort],
                range: bool_sort,
            }
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_overloads_are_collected() {
        // Appendix B declares `max` for Pos # Nat, Nat # Pos and Nat # Nat
        // (and more through Int), all collected as one overloaded name.
        let (_, ctx) = resolve("map f: Nat;");
        let signature = ctx.signature.as_ref().unwrap();
        assert!(signature.mappings["max"].len() >= 3);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_constructor_for_basic_sort_is_allowed_when_trusted() {
        // The system-defined specification declares constructors for basic
        // sorts on purpose (`@c0: Nat`) — `push_declarations`'s `trusted`
        // parameter is what exempts this, the one signature rule trusted
        // content legitimately breaks.
        let spec = DataSpecification::from_untyped(UntypedDataSpecification::parse("map f: Bool;").unwrap()).unwrap();
        let mut ctx = TypeCheckContext::new();
        crate::build_signature(&mut ctx, spec.data_specification()).unwrap();

        let system = UntypedDataSpecification::parse("cons @c0: Nat;").unwrap();
        resolve_system_signature(&mut ctx, spec.data_specification(), &system)
            .expect("a constructor for a basic sort is legitimate in the system spec");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_constructor_for_function_sort_is_rejected_even_when_trusted() {
        // Unlike the basic-sort rule, this one is not exempted for trusted
        // content: no template legitimately declares a function-sort
        // constructor, so this only ever fires on an editing mistake.
        let spec = DataSpecification::from_untyped(UntypedDataSpecification::parse("map f: Bool;").unwrap()).unwrap();
        let mut ctx = TypeCheckContext::new();
        crate::build_signature(&mut ctx, spec.data_specification()).unwrap();

        let system = UntypedDataSpecification::parse("cons c: Bool -> (Nat -> Bool);").unwrap();
        let err = resolve_system_signature(&mut ctx, spec.data_specification(), &system)
            .expect_err("a constructor targeting a function sort must be rejected");
        assert!(
            matches!(err, WellTypedError::ConstructorForFunctionSort { ref sort, .. } if sort == "(Nat -> Bool)"),
            "{err}"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_template_instantiation_carries_user_sorts() {
        // `resolve_sort`'s handling of a template-substituted `Resolved` node,
        // exercised directly: production only ever feeds `resolve_system_signature`
        // the basic-sort spec (see its doc comment) — a container instantiation
        // is never part of `system_defined_specification()` at all any more,
        // generated only at lowering time (see `docs/typecheck.md`'s
        // monomorphization-to-lowering milestone) — so this builds the
        // container-instantiated content directly via
        // `build_system_defined_specification`, in an isolated context, to
        // check the substitution logic itself. The list template instantiated
        // with the user sort `D` should resolve `|>` to `D # List(D) -> List(D)`.
        let spec = DataSpecification::from_untyped(
            UntypedDataSpecification::parse("sort D = struct s; map f: List(D);").unwrap(),
        )
        .unwrap();
        let mut sources = SourceMap::new();
        let basics = basic_sort_data_specification(&mut sources, NumberEncoding::Binary);
        let (mut system, _) =
            build_system_defined_specification(&mut sources, spec.data_specification(), basics, NumberEncoding::Binary);
        // Unlike the real lowering-time call (which seeds the worklist empty, since `basics`'s
        // own content is resolved separately — see `ir::mcrl2_lowering`), this seeds it with the
        // real `basics` to also exercise `|>`'s own container-template output, so `system` here
        // still carries basics's own unresolved `@NatPair`/`@word` references; resolve them the
        // same way `from_untyped_with` resolves `system` against the shared `sorts` table.
        crate::apply_sorts_in_spec(&mut system, |sort| crate::resolve_sort_id(sort, spec.sorts())).unwrap();

        let mut ctx = TypeCheckContext::new();
        crate::build_signature(&mut ctx, spec.data_specification()).unwrap();
        resolve_system_signature(&mut ctx, spec.data_specification(), &system).unwrap();

        let def = SortId::new(*spec.sorts().index("D").unwrap());
        let d = ctx.sorts.def(def);
        let d_list = ctx.sorts.generic(ComplexSort::List, d);
        let expected = ctx.sorts.function(vec![d, d_list], d_list);

        let signature = ctx.signature.as_ref().unwrap();
        assert!(signature.constructors["|>"].contains(&expected));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_system_internal_sort_gets_fresh_def() {
        // `@NatPair` is folded into the shared `sort_declarations` table by
        // `from_untyped_with` (see `docs/typecheck.md`'s `DefId`-offset
        // milestone), so it has an ordinary `SortId` findable by name, and
        // `sort_name` recovers it the same way it would a user sort.
        let (spec, ctx) = resolve("sort D; map f: D;");
        let signature = ctx.signature.as_ref().unwrap();

        let pair_constructor = signature.constructors["@cPair"][0];
        let ResolvedSort::Function { domain: _, range } = ctx.sorts.get(pair_constructor) else {
            panic!("expected a function sort");
        };
        let ResolvedSort::Def(def) = ctx.sorts.get(*range) else {
            panic!("expected a nominal sort");
        };
        assert_eq!(*def, SortId::new(*spec.sorts().index("@NatPair").unwrap()));
        assert_eq!(ctx.sort_name(spec.data_specification(), *def), Some("@NatPair"));
    }

    /// Type checks `text` through the full pipeline.
    fn resolve_full(text: &str) -> DataSpecification {
        DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()).unwrap()
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_full_signature_covers_containers() {
        // `in`/`@setfset` resolve as schemes in the one pooled signature, the
        // same way for every instantiation — there is no more per-group
        // signature to check instead.
        let spec = resolve_full("map f: Set(Nat);");
        let ctx = spec.context();
        let signature = ctx.signature.as_ref().unwrap();
        assert!(
            signature.schemes.contains_key("in") && signature.schemes.contains_key("@setfset"),
            "the pooled signature must resolve 'in'/'@setfset' as schemes for a spec using Set(Nat)"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_full_signature_validates_equation_binder_sorts() {
        // `Set` pulls in the `forall c:S. ...` extensionality equation, whose
        // binder sort must resolve; `from_untyped` fails otherwise.
        let spec = resolve_full("map f: Set(Nat);");
        assert!(
            !spec.system_defined_specification().equation_declarations.is_empty(),
            "the Set template should contribute equations to walk"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_merge_signatures_unions_overloads_by_name() {
        let a = Signature {
            constructors: HashMap::from([("c".to_string(), vec![ResolvedSortId::new(0)])]),
            ..Signature::default()
        };
        let b = Signature {
            constructors: HashMap::from([("@cPair".to_string(), vec![ResolvedSortId::new(1)])]),
            ..Signature::default()
        };
        let merged = merge_signatures(&a, &b);
        assert!(merged.constructors.contains_key("c"));
        assert!(merged.constructors.contains_key("@cPair"));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_struct_desugared_symbols_resolve_in_their_own_group_signature() {
        // `c1`/`is_c1` are declared on the user spec by struct desugaring, not
        // on `system`, yet must still resolve in their own isolated override.
        let spec = resolve_full("sort D = struct c1(pr1: Nat)?is_c1; map f: Set(D);");
        let ctx = spec.context();
        assert!(
            !ctx.struct_signature_overrides.is_empty(),
            "the struct's own equations should produce at least one override"
        );
        assert!(
            ctx.struct_signature_overrides
                .values()
                .any(|signature| signature.mappings.contains_key("is_c1")),
            "is_c1's own struct override should see it"
        );
    }
}
