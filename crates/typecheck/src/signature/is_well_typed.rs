use std::collections::HashSet;
use std::ops::ControlFlow;

use thiserror::Error;

use merc_syntax::DataExpr;
use merc_syntax::DataExprKind;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::SourceMap;
use merc_syntax::Span;
use merc_syntax::Traverse;
use merc_syntax::UntypedDataSpecification;
use merc_syntax::VarId;
use merc_utilities::MercError;
use merc_utilities::Step;

use crate::InferenceError;
use crate::nonempty_sorts;

/// The post-normalization well-typedness checks of 15.1.7 that `build_signature`
/// does not already cover.
///
/// `build_signature` runs *before* this and rejects — in a stronger,
/// alias-aware form — every signature-level condition the two once shared
/// (constructor/mapping disjointness, products outside a function domain, and
/// constructors for basic or function sorts), so only three genuinely separate
/// checks remain here:
///
/// * equation-variable well-formedness (no duplicate variable in a `var` block,
///   no bare product sort on one), which is not a signature concern;
/// * every `var`-block variable used in an equation's condition or right-hand
///   side occurs in its left-hand side too, so the equation is executable by
///   rewriting — real mCRL2's own type checker has this rule
///   (`data_type_checker::operator()(data_equation_vector&)`), unconditionally
///   before a 2017 simplification (`02ec6305cfc`) nested it inside a branch
///   that only runs when the equation's two sides don't already share a
///   common sort on the first pass — in effect turning it off for the common
///   case, seemingly as an incidental side effect of that simplification
///   rather than a deliberate relaxation (the rewriter still drops such an
///   equation with a warning at a later stage, so the *intent* that it be
///   rejected up front survives even where current upstream mCRL2's
///   type-checker no longer enforces it). This restores the unconditional
///   form; and
/// * sort non-emptiness, which must run on the *normalized* specification —
///   `nonempty_sorts` unifies a sort with its aliases only once alias
///   indirection is expanded, so a sort inhabited only through an alias would
///   otherwise be misreported as empty.
pub(crate) fn is_well_typed(spec: &UntypedDataSpecification) -> Result<(), WellTypedError> {
    for equation in &spec.equation_declarations {
        // Inference resolves a variable by name, so a duplicate would silently
        // shadow the earlier declaration; mCRL2 rejects the block outright.
        let mut names = HashSet::new();
        for var in &equation.variables {
            if !names.insert(var.identifier.as_str()) {
                return Err(WellTypedError::DuplicateEquationVariable {
                    variable: var.identifier.node.clone(),
                    span: var.identifier.span.clone(),
                });
            }
            // A product sort only has meaning as the domain of a function sort.
            check_products_within_domains(&var.sort)?;
        }

        let declared: HashSet<VarId> = equation.variables.iter().filter_map(|var| var.var_id).collect();
        for eqn in &equation.equations {
            check_variables_occur_on_lhs(&declared, eqn.condition.as_ref(), &eqn.lhs, &eqn.rhs)?;
        }
    }

    // Check that all sorts are syntactically non-empty. `nonempty_sorts` already
    // assumes sorts without constructors (abstract sorts and aliases) to be
    // non-empty, so only genuine constructor sorts are reported here, as in
    // mCRL2's check_for_empty_constructor_domains.
    let nonempty = nonempty_sorts(spec);
    for sort in &spec.sort_declarations {
        let id = sort.id.expect("The sorts must be resolved");
        if !nonempty.contains(&id) {
            return Err(WellTypedError::EmptySort {
                sort: sort.identifier.clone(),
                span: sort.span.clone(),
            });
        }
    }

    Ok(())
}

/// Every occurrence of one of `declared` (the enclosing equation block's own `var`-block
/// variables) reachable in `condition`/`rhs` must also occur somewhere in `lhs` — otherwise
/// rewriting `lhs` to `rhs` would leave a variable in the result with no binding to draw a value
/// from. A `lambda`/`forall`/`exists`/comprehension binder introduces its own, distinct `VarId`
/// (assigned by `resolve_data_specification_variables` before this ever runs), so walking the
/// whole subtree — including inside such a binder's own body — cannot mistake a locally-bound
/// name for one of `declared`.
fn check_variables_occur_on_lhs(
    declared: &HashSet<VarId>,
    condition: Option<&DataExpr>,
    lhs: &DataExpr,
    rhs: &DataExpr,
) -> Result<(), WellTypedError> {
    let lhs_vars = collect_declared_var_occurrences(declared, lhs);

    for expr in condition.into_iter().chain(std::iter::once(rhs)) {
        if let Some((name, span)) = expr.visit(|node| match &node.node {
            DataExprKind::Resolved(name, id) if declared.contains(id) && !lhs_vars.contains(id) => {
                ControlFlow::Break((name.clone(), node.span.clone()))
            }
            _ => ControlFlow::Continue(()),
        }) {
            return Err(WellTypedError::UnboundEquationVariable { variable: name, span });
        }
    }
    Ok(())
}

/// Every `VarId` in `declared` that occurs (as a `Resolved` node) anywhere in `expr`.
fn collect_declared_var_occurrences(declared: &HashSet<VarId>, expr: &DataExpr) -> HashSet<VarId> {
    let mut found = HashSet::new();
    expr.visit::<std::convert::Infallible, _>(|node| {
        if let DataExprKind::Resolved(_, id) = &node.node
            && declared.contains(id)
        {
            found.insert(*id);
        }
        ControlFlow::Continue(())
    });
    found
}

#[derive(Debug, Error)]
pub enum WellTypedError {
    #[error("Constructor '{}' and mapping '{}' have the same identifier", constructor, map)]
    ConstructorAndMappingConflict {
        constructor: String,
        map: String,
        span: Span,
    },

    #[error("Zero-arity constant '{}' is declared more than once with different sorts", name)]
    DuplicateConstantDifferentSort { name: String, span: Span },

    #[error("'{}' redeclares a system-defined function", name)]
    SystemFunctionRedeclared { name: String, span: Span },

    #[error(
        "Constructors cannot be defined for basic sorts, but constructor '{}' is defined for sort '{}'",
        constructor,
        sort
    )]
    ConstructorForBasicSort {
        constructor: String,
        sort: String,
        span: Span,
    },

    #[error(
        "Constructors cannot be defined for function sorts, but constructor '{}' is defined for sort '{}'",
        constructor,
        sort
    )]
    ConstructorForFunctionSort {
        constructor: String,
        sort: String,
        span: Span,
    },

    #[error("Sort '{}' is syntactically empty", sort)]
    EmptySort { sort: String, span: Span },

    #[error("A product sort '{}' may only appear as the domain of a function sort", sort)]
    ProductSortOutsideFunctionDomain { sort: String, span: Span },

    #[error("The variable '{}' occurs multiple times in a var block", variable)]
    DuplicateEquationVariable { variable: String, span: Span },

    #[error(
        "The variable '{}' occurs in the equation's condition or right-hand side, but not in its left-hand side",
        variable
    )]
    UnboundEquationVariable { variable: String, span: Span },

    #[error("Alias cycle detected: {:?}", sorts)]
    AliasCycle { sorts: Vec<String>, span: Span },

    #[error("Sort '{sort}' is recursively defined via a function sort, or a set or a bag type container")]
    RecursiveAliasThroughFunctionSort { sort: String, span: Span },

    #[error("Error: '{0}'")]
    Custom(MercError),

    /// A Phase-3 sort inference error in a user equation.
    #[error(transparent)]
    Inference(#[from] InferenceError),

    // These are name resolution errors, but we include them here to avoid having to define a separate error type for name resolution.
    #[error("Duplicate sort declaration: '{}'", sort)]
    DuplicateSortDeclaration { sort: String, span: Span },

    #[error("Undefined sort: '{}'", sort)]
    UndefinedSort { sort: String, span: Span },

    #[error("Duplicate type variable declaration: '{}'", type_var)]
    DuplicateTypeVarDeclaration { type_var: String, span: Span },

    #[error("Undefined type variable: '{}'", type_var)]
    UndefinedTypeVar { type_var: String, span: Span },
}

impl WellTypedError {
    /// The span of the offending sub-expression, for the variants that carry
    /// one. Only [WellTypedError::Custom] has no source location to point at.
    pub fn span(&self) -> Option<&Span> {
        match self {
            WellTypedError::Inference(error) => Some(error.span()),
            WellTypedError::ConstructorAndMappingConflict { span, .. }
            | WellTypedError::DuplicateConstantDifferentSort { span, .. }
            | WellTypedError::SystemFunctionRedeclared { span, .. }
            | WellTypedError::ConstructorForBasicSort { span, .. }
            | WellTypedError::ConstructorForFunctionSort { span, .. }
            | WellTypedError::EmptySort { span, .. }
            | WellTypedError::ProductSortOutsideFunctionDomain { span, .. }
            | WellTypedError::DuplicateEquationVariable { span, .. }
            | WellTypedError::UnboundEquationVariable { span, .. }
            | WellTypedError::AliasCycle { span, .. }
            | WellTypedError::RecursiveAliasThroughFunctionSort { span, .. }
            | WellTypedError::DuplicateSortDeclaration { span, .. }
            | WellTypedError::UndefinedSort { span, .. }
            | WellTypedError::DuplicateTypeVarDeclaration { span, .. }
            | WellTypedError::UndefinedTypeVar { span, .. } => Some(span),
            WellTypedError::Custom(_) => None,
        }
    }

    /// Renders this error's message, followed by a caret-annotated source
    /// snippet (see [merc_syntax::Span::render]) when a span is available.
    /// `sources` must contain the original specification text the error was
    /// raised against.
    pub fn render(&self, sources: &SourceMap) -> String {
        match self.span() {
            Some(span) => format!("{self}\n{}", span.render(sources)),
            None => self.to_string(),
        }
    }
}

/// Checks that every product sort occurs as (part of the spine of) a function
/// sort's domain, the only position where `A # B` has meaning.
///
/// The domain and range of a `Function` need different treatment, which the
/// visitor context cannot express (all children receive the same context), so
/// that case is handled manually and pruned.
pub(crate) fn check_products_within_domains(sort: &SortExpression) -> Result<(), WellTypedError> {
    sort.visit_with::<(), (), WellTypedError, _>((), |expr, ()| match &expr.node {
        SortExpressionKind::Product { .. } => Err(WellTypedError::ProductSortOutsideFunctionDomain {
            sort: expr.to_string(),
            span: expr.span.clone(),
        }),
        SortExpressionKind::Function { domain, range } => {
            check_product_spine(domain)?;
            check_products_within_domains(range)?;
            Ok(ControlFlow::Continue(Step::Prune))
        }
        _ => Ok(ControlFlow::Continue(Step::Into(()))),
    })
    .map(|_| ())
}

/// Walks the `Product` spine of a function domain, where products are the
/// domain separator, and checks the leaf sorts.
fn check_product_spine(sort: &SortExpression) -> Result<(), WellTypedError> {
    match &sort.node {
        SortExpressionKind::Product { lhs, rhs } => {
            check_product_spine(lhs)?;
            check_product_spine(rhs)
        }
        _ => check_products_within_domains(sort),
    }
}

/// Returns whether a binder sort inside an equation body is a valid variable
/// sort. `hoist_anonymous_structs` hoists an anonymous `struct` on a binder
/// into a named declaration like any other occurrence, so the only shape this
/// rejects is a bare product sort, which is not a sort at all — a construct
/// binding one is rejected during inference (see `binder_sort`).
pub(crate) fn is_supported_binder_sort(sort: &SortExpression) -> bool {
    check_products_within_domains(sort).is_ok()
}

#[cfg(test)]
mod tests {
    use merc_syntax::UntypedDataSpecification;

    use crate::DataSpecification;
    use crate::WellTypedError;

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_well_typed_spec() {
        let spec = UntypedDataSpecification::parse(
            "
            sort D;
            cons f: D -> Nat;
        ",
        )
        .unwrap();

        match DataSpecification::from_untyped(spec) {
            Err(WellTypedError::ConstructorForBasicSort { constructor, sort, .. })
                if constructor == "f" && sort == "Nat" => {}
            Err(other) => panic!("Unexpected error {:?}", other),
            _ => panic!("Expected from_untyped to fail"),
        }
    }

    /// Inference resolves variables by name, so without this check a
    /// duplicate would win by declaration order: `var n: Bool; n: Nat;` was
    /// accepted (the later `n: Nat` shadowing the earlier declaration) while
    /// the swapped order was rejected with a misleading no-typing error.
    /// mCRL2 rejects both ("The variable n occurs multiple times").
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_duplicate_equation_variable_is_rejected() {
        for text in [
            "map f: Nat -> Bool; var n: Bool; n: Nat; eqn f(n) = true;",
            "map f: Nat -> Bool; var n: Nat; n: Bool; eqn f(n) = true;",
        ] {
            let spec = UntypedDataSpecification::parse(text).unwrap();
            match DataSpecification::from_untyped(spec) {
                Err(WellTypedError::DuplicateEquationVariable { variable, .. }) if variable == "n" => {}
                Err(other) => panic!("Unexpected error {:?}", other),
                _ => panic!("Expected from_untyped to fail"),
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_abstract_sort_is_allowed() {
        let spec = UntypedDataSpecification::parse(
            "
            sort D;
            map f: D -> D;
        ",
        )
        .unwrap();

        DataSpecification::from_untyped(spec).expect("a sort without constructors is assumed non-empty");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_product_sort_outside_function_domain_is_rejected() {
        for text in [
            "map f: Pos -> (Pos # Pos);",
            "map f: List(Pos # Pos);",
            "sort D = Bool # Bool;",
            "map f: Nat; var x: Nat # Nat; eqn f = 0;",
            "map f: ((Pos # Pos) -> Bool) -> (Nat # Nat);",
        ] {
            match DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()) {
                Err(WellTypedError::ProductSortOutsideFunctionDomain { .. }) => {}
                Err(other) => panic!("unexpected error {other:?} for {text}"),
                Ok(_) => panic!("expected {text} to be rejected"),
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_product_sort_in_function_domain_is_allowed() {
        // Parenthesized products inside a domain are still domain separators,
        // including in a nested higher-order function sort.
        for text in [
            "map f: (Pos # Pos) # Pos -> Bool;",
            "map f: ((Pos # Pos) -> Bool) -> Bool;",
        ] {
            DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap())
                .unwrap_or_else(|err| panic!("expected {text} to typecheck, got {err}"));
        }
    }
}
