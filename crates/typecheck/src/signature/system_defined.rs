use std::collections::BTreeSet;
use std::collections::HashSet;

use merc_syntax::SortExpressionKind;
use merc_syntax::UntypedDataSpecification;

use crate::SortCollectionMode;
use crate::WellTypedError;
use crate::collect_system_sorts_in_spec;
use crate::is_system_generated_name;
use crate::polymorphic_operator_names;

/// Any user `cons`/`map` declaration whose name collides with a system-defined
/// function is rejected.
///
/// Also rejects `@`-prefixed names outright.
pub(crate) fn check_no_system_function_redeclaration(
    spec: &UntypedDataSpecification,
    system: &UntypedDataSpecification,
) -> Result<(), WellTypedError> {
    // Gather the constructors and mappings from system, and added the polymorphic operator names.
    let mut reserved: HashSet<&str> = HashSet::new();
    reserved.extend(
        system
            .constructor_declarations
            .iter()
            .map(|decl| decl.identifier.as_str()),
    );
    reserved.extend(system.map_declarations.iter().map(|decl| decl.identifier.as_str()));
    let reserved_polymorphic: HashSet<&'static str> = polymorphic_operator_names().collect();

    for decl in &spec.constructor_declarations {
        if reserved.contains(decl.identifier.as_str())
            || reserved_polymorphic.contains(decl.identifier.as_str())
            || is_system_generated_name(&decl.identifier)
        {
            return Err(WellTypedError::SystemFunctionRedeclared {
                name: decl.identifier.node.clone(),
                span: decl.identifier.span.clone(),
            });
        }
    }

    for decl in &spec.map_declarations {
        if reserved.contains(decl.identifier.as_str())
            || reserved_polymorphic.contains(decl.identifier.as_str())
            || is_system_generated_name(&decl.identifier)
        {
            return Err(WellTypedError::SystemFunctionRedeclared {
                name: decl.identifier.node.clone(),
                span: decl.identifier.span.clone(),
            });
        }
    }
    Ok(())
}

/// Every distinct function-update arity `spec` needs.
pub(crate) fn function_update_arities(spec: &UntypedDataSpecification) -> BTreeSet<usize> {
    let mut worklist = Vec::new();

    collect_system_sorts_in_spec(spec, &mut worklist, SortCollectionMode::ContainersAndFunctions);
    worklist
        .into_iter()
        .filter_map(|sort| match &sort.node {
            SortExpressionKind::Function { .. } => Some(1),
            SortExpressionKind::FlattenedFunction { domain, .. } => Some(domain.len()),
            _ => None,
        })
        .collect()
}
