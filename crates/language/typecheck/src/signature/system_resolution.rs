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
    //! The system-defined specification is trusted to declare constructors for basic sorts, but
    //! not for function sorts; these specifications are crafted, the bundled ones are always
    //! valid.

    use merc_syntax::UntypedDataSpecification;

    use crate::DataSpecification;
    use crate::TypeCheckContext;
    use crate::WellTypedError;
    use crate::resolve_system_signature;

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_constructor_for_basic_sort_is_allowed_when_trusted() {
        // The system-defined specification declares constructors for basic sorts on purpose.
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
}
