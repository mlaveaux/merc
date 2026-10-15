use std::collections::HashMap;
use std::sync::Arc;

use merc_syntax::UntypedDataSpecification;

use crate::CONTAINER_TEMPLATES;
use crate::PolySortScheme;
use crate::ResolvedSortId;
use crate::Signature;
use crate::TypeCheckContext;
use crate::WellTypedError;
use crate::push_declarations;
use crate::push_overload;
use crate::resolve_sort;

/// Resolves the constructor and mapping declarations of the system-defined
/// specification onto the interned sort lattice, merging them into
/// `ctx.signature`.
///
/// Every `Reference` node of `system`'s own declarations must already be
/// resolved.
///
/// Requires `build_signature` to have already populated `ctx.signature` with
/// the user's own declarations, so there is something to merge into.
pub(crate) fn resolve_system_signature(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    system: &UntypedDataSpecification,
) -> Result<(), WellTypedError> {
    let mut signature = Signature::default();
    let mut constants: HashMap<String, ResolvedSortId> = HashMap::new();
    push_declarations(ctx, system, spec, true, &mut signature, &mut constants)?;

    let merged = merge_signatures(
        ctx.signature
            .as_deref()
            .expect("build_signature ran before resolve_system_signature"),
        &signature,
    );
    ctx.signature = Some(Arc::new(merged));
    Ok(())
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
/// the template's own `type_var` block interns to the same [`ResolvedSort::TypeVar`](crate::ResolvedSort::TypeVar),
/// on the same footing as any other lattice element.
///
/// Safe to call with any of [CONTAINER_TEMPLATES]/`crate::BUILTIN_SCHEME_TEMPLATE`:
/// none of them contains a `Resolved(_, SortId)` node or a nominal `sort X;`
/// declaration (only `type_var`, primitive, container and function sorts), so
/// there is no `SortId` to resolve and hence no risk of it being looked up
/// against the wrong spec's `sort_declarations`.
///
/// This is the one shared mechanism behind `ctx.signature`'s `schemes` (containers,
/// function-update and the comparison/`if` builtins) — see `build_signature`. Every role
/// (`User`/`Template`/`System`) resolves a name's polymorphic overloads from this same table; there
/// is no separate, narrower scheme table for struct-scoped system equations any more.
pub(crate) fn build_polymorphic_schemes<'a, I>(
    ctx: &mut TypeCheckContext,
    templates: I,
) -> HashMap<String, Vec<PolySortScheme>>
where
    I: IntoIterator<Item = &'a UntypedDataSpecification>,
{
    let mut schemes: HashMap<String, Vec<PolySortScheme>> = HashMap::new();
    for template in templates {
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
                .push(PolySortScheme { sort: resolved });
        }
    }
    schemes
}

/// The reserved names of every polymorphic built-in operator.
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
        // generated only at lowering time — so this builds the
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
        // `from_untyped_with`, so it has an ordinary `SortId` findable by
        // name, and `sort_name` recovers it the same way it would a user sort.
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
    fn test_struct_desugared_symbols_resolve_in_the_pooled_signature() {
        // `c1`/`is_c1` are declared on the user spec by struct desugaring, not on `system`, yet
        // must still resolve — in the one pooled `ctx.signature`, same as any other declaration.
        let spec = resolve_full("sort D = struct c1(pr1: Nat)?is_c1; map f: Set(D);");
        let ctx = spec.context();
        assert!(
            ctx.signature.as_ref().unwrap().mappings.contains_key("is_c1"),
            "is_c1 should resolve in the pooled signature"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_struct_nullary_constant_colliding_with_unrelated_struct_function_is_rejected() {
        // Confirmed bug 1 (`docs/typecheck-struct-system-unification-plan.md`): a nullary
        // constructor of one struct sharing a literal name with an unrelated struct's ≥1-ary
        // constructor makes the whole specification rejected with `AmbiguousExpression` on struct
        // A's own generated, unconstrained reflexivity equation `a == a = true` — `==` is
        // polymorphic over any single sort, so once B's unrelated `a: Nat -> B` is pooled in there
        // are two self-consistent readings with nothing to prefer one over the other.
        //
        // A per-struct signature scoping fix existed for this (see the plan's "Bug 1"/"Bug 2"), but
        // was removed: it special-cased compiler-generated equations rather than fixing name
        // resolution in general, so the byte-identical collision written by hand (see
        // `test_user_written_equivalent_of_bug_1_is_still_rejected`) was rejected regardless. This
        // is now a known, accepted regression, left failing rather than routed around by scoping.
        let result = DataSpecification::from_untyped(
            UntypedDataSpecification::parse(
                "sort A = struct a?is_a; \
                 B = struct a(x: Nat)?is_b;",
            )
            .unwrap(),
        );
        assert!(
            result.is_err(),
            "expected struct A's own reflexivity equation to be rejected as ambiguous now that \
             struct equations pool with the full signature unfiltered"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_two_structs_sharing_same_arity_constructor_name_is_rejected() {
        // Confirmed bug 2 (`docs/typecheck-struct-system-unification-plan.md`): two unrelated
        // structs whose constructors share a name at the same ≥1 arity (differing only in
        // codomain) make the whole specification rejected with `AmbiguousExpression` on struct C's
        // own generated equality equation `c(@x0_0) == c(@y0_0) = @x0_0 == @y0_0`, for the same
        // pooling reason as bug 1 above — see that test's doc comment for why this is now a known,
        // accepted regression rather than one routed around by per-struct signature scoping.
        let result = DataSpecification::from_untyped(
            UntypedDataSpecification::parse(
                "sort C = struct c(v: Nat)?is_c; \
                 D = struct c(w: Nat)?is_d;",
            )
            .unwrap(),
        );
        assert!(
            result.is_err(),
            "expected struct C's own equality equation to be rejected as ambiguous now that \
             struct equations pool with the full signature unfiltered"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_user_written_equivalent_of_bug_1_is_still_rejected() {
        // The byte-identical collision to `test_struct_nullary_constant_colliding_with_unrelated_struct_function_is_rejected`,
        // written by hand as a user equation instead of generated for a struct — resolves against
        // the full pooled signature exactly the same way, and is rejected exactly the same way.
        // Real mCRL2 accepts both this and the struct-generated form via a non-backtracking
        // heuristic that is not actually sound in general — see
        // `docs/typecheck-struct-system-unification-plan.md`'s "Bug 1" root-cause section.
        let result = DataSpecification::from_untyped(
            UntypedDataSpecification::parse(
                "sort A = struct a?is_a; \
                 B = struct a(x: Nat)?is_b; \
                 map t: Bool; \
                 eqn t = (a == a);",
            )
            .unwrap(),
        );
        assert!(result.is_err(), "expected this collision to be rejected as ambiguous");
    }
}
