use std::collections::HashSet;

use log::debug;

use merc_collections::IndexedSet;
use merc_syntax::ConstructorId;
use merc_syntax::DataExpr;
use merc_syntax::DataExprKind;
use merc_syntax::EqnSpec;
use merc_syntax::EqnSpecId;
use merc_syntax::EquationId;
use merc_syntax::MapId;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::SortId;
use merc_syntax::Traverse;
use merc_syntax::TypeVarId;
use merc_syntax::UntypedDataSpecification;

use crate::WellTypedError;

/// Assigns unique [TypeVarId]s to all `type_var` declarations, rewrites every
/// [SortExpressionKind::Reference] naming one of them into a [SortExpressionKind::TypeVar]
/// throughout the specification, and then resolves all `TypeVar` nodes to their id. Returns an
/// indexed set that indicates the mapping from type-variable identifiers to their [TypeVarId]s.
pub(crate) fn resolve_type_variables(
    spec: &mut UntypedDataSpecification,
) -> Result<IndexedSet<String>, WellTypedError> {
    let mut vars = IndexedSet::new();

    for (i, decl) in spec.type_var_declarations.iter_mut().enumerate() {
        decl.id = Some(TypeVarId::new(i));
        debug!("type variable '{}' declared as id {i}", decl.identifier);

        if !vars.insert(decl.identifier.clone()).1 {
            return Err(WellTypedError::DuplicateTypeVarDeclaration {
                type_var: decl.identifier.clone(),
                span: decl.span.clone(),
            });
        }
    }

    if !vars.is_empty() {
        let names: HashSet<&str> = spec
            .type_var_declarations
            .iter()
            .map(|decl| decl.identifier.as_str())
            .collect();

        for sort in &mut spec.sort_declarations {
            if let Some(expr) = &mut sort.expr {
                resolve_type_var_name(expr, &names);
            }
        }

        for constructor in &mut spec.constructor_declarations {
            resolve_type_var_name(&mut constructor.sort, &names);
        }

        for map in &mut spec.map_declarations {
            resolve_type_var_name(&mut map.sort, &names);
        }

        for equation in &mut spec.equation_declarations {
            resolve_type_var_names_in_equation(equation, &names);
        }
    }

    apply_sorts_in_spec(spec, |sort| resolve_type_var_id(sort, &vars))?;

    Ok(vars)
}

/// Rewrites every `Reference` in `sort` naming one of `names` into a `TypeVar`.
fn resolve_type_var_name(sort: &mut SortExpression, names: &HashSet<&str>) {
    sort.transform(|expr| {
        if let SortExpressionKind::Reference(name) = &expr.node
            && names.contains(name.as_str())
        {
            expr.node = SortExpressionKind::TypeVar(name.clone());
        }
    });
}

/// Rewrites the binder sorts of a single `var ... eqn ...` block: its declared
/// variables and its equations' conditions, left- and right-hand sides.
fn resolve_type_var_names_in_equation(equation: &mut EqnSpec, names: &HashSet<&str>) {
    for var in &mut equation.variables {
        resolve_type_var_name(&mut var.sort, names);
    }

    for eqn in &mut equation.equations {
        if let Some(condition) = &mut eqn.condition {
            resolve_type_var_names_in_expr(condition, names);
        }

        resolve_type_var_names_in_expr(&mut eqn.lhs, names);
        resolve_type_var_names_in_expr(&mut eqn.rhs, names);
    }
}

/// See [resolve_type_var_name]; applied to every binder sort (lambda, quantifier and set/bag
/// comprehension variables) inside a data expression.
fn resolve_type_var_names_in_expr(expr: &mut DataExpr, names: &HashSet<&str>) {
    expr.transform(|expr| match &mut expr.node {
        DataExprKind::Lambda { variables, body: _ }
        | DataExprKind::Quantifier {
            op: _,
            variables,
            body: _,
        } => {
            for variable in variables {
                resolve_type_var_name(&mut variable.sort, names);
            }
        }
        DataExprKind::SetBagComp { variable, predicate: _ } => {
            resolve_type_var_name(&mut variable.sort, names);
        }
        _ => {}
    });
}

/// Rewrites every `TypeVar` node of `sort` to `ResolvedTypeVar(TypeVarId)` using the type-variable
/// name index built by [resolve_type_variables], or fails on a name that names no declared type
/// variable (which should not arise from parsing, but a hand-built specification could still
/// construct one).
fn resolve_type_var_id(sort: &SortExpression, resolved: &IndexedSet<String>) -> Result<SortExpression, WellTypedError> {
    sort.clone().apply(|expr| {
        if let SortExpressionKind::TypeVar(name) = &expr.node {
            if let Some(id) = resolved.index(name) {
                return Ok(Some(
                    SortExpressionKind::ResolvedTypeVar(TypeVarId::new(*id)).spanned(expr.span.clone()),
                ));
            }

            return Err(WellTypedError::UndefinedTypeVar {
                type_var: name.clone(),
                span: expr.span.clone(),
            });
        }

        Ok(None)
    })
}

/// Assigns unique DefIds to all sort declarations, and then resolves all sort
/// expressions to their id. Returns an indexed set that indicates the mapping
/// from sort identifiers to their DefIds.
pub(crate) fn resolve_sort_ids(spec: &mut UntypedDataSpecification) -> Result<IndexedSet<String>, WellTypedError> {
    // Byte-identical sort declarations are deduplicated.
    let mut seen = HashSet::new();
    let before = spec.sort_declarations.len();
    spec.sort_declarations
        .retain(|decl| seen.insert((decl.identifier.clone(), decl.expr.clone())));
    if spec.sort_declarations.len() < before {
        debug!(
            "deduplicated {} identical sort declaration(s)",
            before - spec.sort_declarations.len()
        );
    }

    // Every sort declaration should have a unique name.
    let mut sorts = IndexedSet::new();

    // Assign unique IDs to all sort declarations
    for (i, sort) in spec.sort_declarations.iter_mut().enumerate() {
        sort.id = Some(SortId::new(i));
        debug!("sort '{}' declared as id {i}", sort.identifier);

        if !sorts.insert(sort.identifier.clone()).1 {
            return Err(WellTypedError::DuplicateSortDeclaration {
                sort: sort.identifier.clone(),
                span: sort.span.clone(),
            });
        }
    }

    // Resolve all sorts.
    apply_sorts_in_spec(spec, |sort| resolve_sort_id(sort, &sorts))?;

    Ok(sorts)
}

/// Assigns a unique id to every constructor declaration, map declaration,
/// equation specification block and equation.
///
/// These lists can grow after name resolution by the desugaring of structured
/// sorts, so this must run after `desugar_structured_sorts`.
pub(crate) fn assign_declaration_ids(spec: &mut UntypedDataSpecification) {
    for (i, decl) in spec.constructor_declarations.iter_mut().enumerate() {
        decl.id = Some(ConstructorId::new(i));
    }

    for (i, decl) in spec.map_declarations.iter_mut().enumerate() {
        decl.id = Some(MapId::new(i));
    }

    for (i, eqn_spec) in spec.equation_declarations.iter_mut().enumerate() {
        eqn_spec.id = Some(EqnSpecId::new(i));
        // Each variable's `VarId` is assigned earlier, by `resolve_data_specification_variables`.
        for (j, equation) in eqn_spec.equations.iter_mut().enumerate() {
            equation.id = Some(EquationId::new(j));
        }
    }
}

// Apply a mapping function to every sort in the specification, including the
// binder sorts inside equation expressions, so the sort passes treat `{ x: S |
// .. }` and `lambda x: S. ..` like any declaration-level sort.
pub(crate) fn apply_sorts_in_spec<E, F>(spec: &mut UntypedDataSpecification, mut f: F) -> Result<(), E>
where
    F: FnMut(&SortExpression) -> Result<SortExpression, E>,
{
    for sort in &mut spec.sort_declarations {
        if let Some(expr) = &sort.expr {
            // Only apply to type aliases.
            sort.expr = Some(f(expr)?);
        }
    }

    for constructor in &mut spec.constructor_declarations {
        constructor.sort = f(&constructor.sort)?;
    }

    for map in &mut spec.map_declarations {
        map.sort = f(&map.sort)?;
    }

    for equation in &mut spec.equation_declarations {
        for var in &mut equation.variables {
            var.sort = f(&var.sort)?;
        }

        for eqn in &mut equation.equations {
            if let Some(condition) = &mut eqn.condition {
                apply_sorts_in_data_expr(condition, &mut f)?;
            }
            apply_sorts_in_data_expr(&mut eqn.lhs, &mut f)?;
            apply_sorts_in_data_expr(&mut eqn.rhs, &mut f)?;
        }
    }

    Ok(())
}

/// Applies `f` to every binder sort (lambda, quantifier and set/bag
/// comprehension variables) inside a data expression.
///
/// `pub(crate)`: also used by `merc_process` (through `crate::checking`) to resolve the binder sorts of a process-body
/// expression (an action argument, a condition, …) — such an expression never goes through
/// [apply_sorts_in_spec] itself, since it doesn't live in the data specification that walks.
pub(crate) fn apply_sorts_in_data_expr<E, F>(expr: &mut DataExpr, f: &mut F) -> Result<(), E>
where
    F: FnMut(&SortExpression) -> Result<SortExpression, E>,
{
    expr.try_transform(&mut |expr| {
        match &mut expr.node {
            DataExprKind::Lambda { variables, body: _ }
            | DataExprKind::Quantifier {
                op: _,
                variables,
                body: _,
            } => {
                for variable in variables {
                    variable.sort = f(&variable.sort)?;
                }
            }
            DataExprKind::SetBagComp { variable, predicate: _ } => {
                variable.sort = f(&variable.sort)?;
            }
            _ => {}
        }
        Ok(())
    })
}

/// Rewrites every `Reference` node of `sort` to `Resolved(name, SortId)` using
/// the sort-name index built by [resolve_sort_ids], or fails on an undeclared name.
///
/// `pub(crate)`: also used by `merc_process` (through `crate::checking`) to resolve an `act`/`proc`/`glob` sort
/// expression, which never goes through [resolve_sort_ids] itself (that only ever runs over the
/// data specification).
pub(crate) fn resolve_sort_id(
    sort: &SortExpression,
    resolved: &IndexedSet<String>,
) -> Result<SortExpression, WellTypedError> {
    sort.clone().apply(|expr| {
        if let SortExpressionKind::Reference(name) = &expr.node {
            if let Some(id) = resolved.index(name) {
                return Ok(Some(
                    SortExpressionKind::Resolved(name.clone(), SortId::new(*id)).spanned(expr.span.clone()),
                ));
            }

            return Err(WellTypedError::UndefinedSort {
                sort: name.clone(),
                span: expr.span.clone(),
            });
        }

        Ok(None)
    })
}
