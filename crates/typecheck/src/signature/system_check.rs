use std::collections::HashSet;
use std::ops::ControlFlow;

use merc_syntax::DataExpr;
use merc_syntax::DataExprKind;
use merc_syntax::IdDecl;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::Traverse;
use merc_syntax::UntypedDataSpecification;

use crate::WellTypedError;
use crate::builtin_scheme_names;
use crate::check_products_within_domains;

/// Verifies that the generated system-defined specification is internally
/// well-formed.
///
/// Checked, for every declaration and equation of `system`:
///
/// - every sort reference is declared (in `system` or `user_spec`), catching
///   uninstantiated template variables like `S`, and every `Resolved` sort
///   indexes a user sort declaration;
/// - product sorts occur only as function domains, and no structured sort
///   survives.
/// - no `var` block declares a variable twice;
/// - every name in an equation resolves: to a binder or equation variable, a
///   constructor or mapping of `system` or `user_spec`, or a builtin scheme;
/// - the free variables of an equation's condition and right-hand side occur in
///   its left-hand side, so every rule is executable by rewriting.
///
/// The signature-level rules of `build_signature` (no constructor for a function or basic sort,
/// constructor/mapping disjointness, no zero-arity symbol under two different sorts) are not this
/// function's job any more: `resolve_system_signature` now runs `push_declarations` — the same
/// checks `build_signature` runs for the user's own declarations, `trusted` — directly over
/// `system`'s constructor/mapping declarations (in practice always exactly `basics`'s own set: a
/// struct's own constructor/projection/recogniser are *user* declarations from its `sort D = struct
/// ...`, desugared onto `user_spec`, not `system` — `structured_sort_equations` contributes only
/// equations, no declarations of its own).
///
/// Full sort inference over the system equations is not run.
pub(crate) fn check_system_specification(
    user_spec: &UntypedDataSpecification,
    system: &UntypedDataSpecification,
) -> Result<(), WellTypedError> {
    let mut sort_names: HashSet<&str> = HashSet::new();
    sort_names.extend(system.sort_declarations.iter().map(|decl| decl.identifier.as_str()));
    sort_names.extend(user_spec.sort_declarations.iter().map(|decl| decl.identifier.as_str()));

    let mut symbols: HashSet<&str> = HashSet::new();
    symbols.extend(
        system
            .constructor_declarations
            .iter()
            .map(|decl| decl.identifier.as_str()),
    );
    symbols.extend(system.map_declarations.iter().map(|decl| decl.identifier.as_str()));
    symbols.extend(
        user_spec
            .constructor_declarations
            .iter()
            .map(|decl| decl.identifier.as_str()),
    );
    symbols.extend(user_spec.map_declarations.iter().map(|decl| decl.identifier.as_str()));
    // The comparison operators and `if` are polymorphic built-ins, usable in a
    // system equation without a declaration. Inserted one at a time so each
    // `'static` name coerces to the borrowed element lifetime of `symbols`.
    for name in builtin_scheme_names() {
        symbols.insert(name);
    }

    let checker = Checker {
        sort_names,
        symbols,
        user_sort_count: user_spec.sort_declarations.len(),
    };

    for declaration in &system.sort_declarations {
        if let Some(expr) = &declaration.expr {
            checker.check_sort(expr)?;
        }
    }
    for declaration in &system.constructor_declarations {
        checker.check_sort(&declaration.sort)?;
    }
    for declaration in &system.map_declarations {
        checker.check_sort(&declaration.sort)?;
    }

    for eqn_spec in &system.equation_declarations {
        let mut variables = HashSet::new();
        for variable in &eqn_spec.variables {
            if !variables.insert(variable.identifier.as_str()) {
                return Err(WellTypedError::DuplicateEquationVariable {
                    variable: variable.identifier.node.clone(),
                    span: variable.identifier.span.clone(),
                });
            }
            checker.check_sort(&variable.sort)?;
        }

        for equation in &eqn_spec.equations {
            let mut scope = Vec::new();
            let mut lhs_variables = HashSet::new();
            checker.check_expr(&equation.lhs, &variables, &mut scope, &mut lhs_variables)?;

            let mut used = HashSet::new();
            checker.check_expr(&equation.rhs, &variables, &mut scope, &mut used)?;
            if let Some(condition) = &equation.condition {
                checker.check_expr(condition, &variables, &mut scope, &mut used)?;
            }

            if let Some(unbound) = used.iter().find(|name| !lhs_variables.contains(*name)) {
                return Err(custom(format!(
                    "the variable '{unbound}' of the system equation '{equation}' does not occur in its left-hand side"
                )));
            }
        }
    }

    Ok(())
}

fn custom(message: String) -> WellTypedError {
    WellTypedError::Custom(message.into())
}

struct Checker<'a> {
    /// The sort names declared in the system or user specification.
    sort_names: HashSet<&'a str>,
    /// The constructor and mapping names of both specifications, plus the
    /// builtin schemes.
    symbols: HashSet<&'a str>,
    /// A `Resolved` sort substituted from the user specification must index
    /// one of its sort declarations.
    user_sort_count: usize,
}

impl Checker<'_> {
    /// Checks that a sort of the system specification references only declared
    /// sorts, and places products only in function domains.
    fn check_sort(&self, sort: &SortExpression) -> Result<(), WellTypedError> {
        let error = sort.visit(|expr| match &expr.node {
            SortExpressionKind::Reference(name) if !self.sort_names.contains(name.as_str()) => ControlFlow::Break(
                format!("the system-defined specification references the undeclared sort '{name}'"),
            ),
            SortExpressionKind::Resolved(name, id) if **id >= self.user_sort_count => ControlFlow::Break(format!(
                "the resolved sort '{name}' does not index a user sort declaration"
            )),
            SortExpressionKind::Struct { .. } => ControlFlow::Break(format!(
                "the system-defined specification contains the structured sort '{expr}'"
            )),
            _ => ControlFlow::Continue(()),
        });
        if let Some(message) = error {
            return Err(custom(message));
        }

        check_products_within_domains(sort)
    }

    /// Checks the names and binder sorts of an equation expression. `scope`
    /// holds the binder-bound names in shadowing order; a use of an equation
    /// variable from `variables` is recorded in `used`.
    fn check_expr<'e>(
        &self,
        expr: &'e DataExpr,
        variables: &HashSet<&str>,
        scope: &mut Vec<&'e str>,
        used: &mut HashSet<&'e str>,
    ) -> Result<(), WellTypedError> {
        match &expr.node {
            // `Resolved` never occurs in the system-defined specification's own equations, but is
            // grouped with `Id` for exhaustiveness.
            DataExprKind::Id(name) | DataExprKind::Resolved(name, _) => {
                if scope.iter().any(|bound| bound == name) {
                    Ok(())
                } else if variables.contains(name.as_str()) {
                    used.insert(name.as_str());
                    Ok(())
                } else if self.symbols.contains(name.as_str()) {
                    Ok(())
                } else {
                    Err(custom(format!(
                        "the system-defined specification uses the undeclared name '{name}'"
                    )))
                }
            }
            DataExprKind::Number(_)
            | DataExprKind::Bool(_)
            | DataExprKind::EmptyList
            | DataExprKind::EmptySet
            | DataExprKind::EmptyBag => Ok(()),
            DataExprKind::Application { function, arguments } => {
                self.check_expr(function, variables, scope, used)?;
                for argument in arguments {
                    self.check_expr(argument, variables, scope, used)?;
                }
                Ok(())
            }
            DataExprKind::List(elements) | DataExprKind::Set(elements) => {
                for element in elements {
                    self.check_expr(element, variables, scope, used)?;
                }
                Ok(())
            }
            DataExprKind::Bag(elements) => {
                for element in elements {
                    self.check_expr(&element.expr, variables, scope, used)?;
                    self.check_expr(&element.multiplicity, variables, scope, used)?;
                }
                Ok(())
            }
            DataExprKind::SetBagComp { variable, predicate } => {
                self.check_binder(std::slice::from_ref(variable), predicate, variables, scope, used)
            }
            DataExprKind::Lambda { variables: bound, body }
            | DataExprKind::Quantifier {
                op: _,
                variables: bound,
                body,
            } => self.check_binder(bound, body, variables, scope, used),
            DataExprKind::Unary { op: _, expr } => self.check_expr(expr, variables, scope, used),
            DataExprKind::Binary { op: _, lhs, rhs } => {
                self.check_expr(lhs, variables, scope, used)?;
                self.check_expr(rhs, variables, scope, used)
            }
            DataExprKind::FunctionUpdate { expr, update } => {
                self.check_expr(expr, variables, scope, used)?;
                self.check_expr(&update.expr, variables, scope, used)?;
                self.check_expr(&update.update, variables, scope, used)
            }
            DataExprKind::Whr { expr, assignments } => {
                // An assignment's right-hand side is evaluated outside the
                // `whr`; only the body sees the bound names.
                for assignment in assignments {
                    self.check_expr(&assignment.expr, variables, scope, used)?;
                }
                let depth = scope.len();
                scope.extend(assignments.iter().map(|assignment| assignment.identifier.as_str()));
                let result = self.check_expr(expr, variables, scope, used);
                scope.truncate(depth);
                result
            }
        }
    }

    /// Checks the sorts of the bound variables, then the body with them in
    /// scope.
    fn check_binder<'e>(
        &self,
        bound: &'e [IdDecl],
        body: &'e DataExpr,
        variables: &HashSet<&str>,
        scope: &mut Vec<&'e str>,
        used: &mut HashSet<&'e str>,
    ) -> Result<(), WellTypedError> {
        for declaration in bound {
            self.check_sort(&declaration.sort)?;
        }

        let depth = scope.len();
        scope.extend(bound.iter().map(|declaration| declaration.identifier.as_str()));
        let result = self.check_expr(body, variables, scope, used);
        scope.truncate(depth);
        result
    }
}

#[cfg(test)]
mod tests {
    use merc_syntax::UntypedDataSpecification;

    use crate::DataSpecification;
    use crate::WellTypedError;
    use crate::check_system_specification;

    /// Runs the checker on the system specification generated for `text`,
    /// verifying the real templates rather than trusting them.
    fn check_generated(text: &str) {
        let spec = DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()).unwrap();
        check_system_specification(spec.data_specification(), spec.system_defined_specification())
            .unwrap_or_else(|err| panic!("the system specification of '{text}' is malformed: {err}"));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_generated_system_specifications_are_well_formed() {
        for text in [
            // The five basic sorts, always included.
            "map f: Bool;",
            // Each container template, with its transitive dependencies.
            "map f: List(Nat);",
            "map f: Set(Bool);",
            "map f: Bag(Nat);",
            // The function-update template, single- and multi-argument.
            "map f: Nat -> Bool;",
            "map f: Nat # Bool -> Nat;",
            // The desugared-struct equations, over a user sort.
            "sort D = struct c(pr: Nat, other: Bool)?is_c | d;",
            // A comprehension contributes Set and Bag for its element sort.
            "map f: Bool; eqn f = 1 in { n: Pos | n < 3 };",
            // A container over a user sort substitutes resolved sorts into
            // the templates.
            "sort D = struct s; map f: List(D);",
        ] {
            check_generated(text);
        }
    }

    /// Checks a hand-written "system" specification against an empty user
    /// specification, seeding the defect the checker must catch.
    fn check_broken(system_text: &str) -> WellTypedError {
        let system = UntypedDataSpecification::parse(system_text).unwrap();
        check_system_specification(&UntypedDataSpecification::default(), &system)
            .expect_err("the specification should be rejected")
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_uninstantiated_template_variable_is_rejected() {
        // An unsubstituted template variable is exactly what a broken
        // `standard_sort` instantiation would leave behind.
        let err = check_broken("map f: List(S);");
        assert!(err.to_string().contains("'S'"), "{err}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_constructor_for_basic_sort_is_allowed() {
        // The system spec declares constructors for basic sorts on purpose
        // (`@c0: Nat`) — `check_system_specification` no longer runs the
        // signature-level rules at all (see its module doc comment), so this
        // just confirms the name/scope walk itself has no opinion on it.
        let system = UntypedDataSpecification::parse("cons @c0: Nat;").unwrap();
        check_system_specification(&UntypedDataSpecification::default(), &system)
            .expect("a constructor for a basic sort is legitimate in the system spec");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_undeclared_equation_symbol_is_rejected() {
        let err = check_broken("map f: Bool; eqn f = g;");
        assert!(err.to_string().contains("'g'"), "{err}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_unbound_right_hand_side_variable_is_rejected() {
        let err = check_broken("map f: Nat -> Nat; var n, m: Nat; eqn f(n) = m;");
        assert!(err.to_string().contains("'m'"), "{err}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_unbound_condition_variable_is_rejected() {
        let err = check_broken("map f: Nat -> Nat; var n, m: Nat; eqn m < n -> f(n) = n;");
        assert!(err.to_string().contains("'m'"), "{err}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_duplicate_equation_variable_is_rejected() {
        let err = check_broken("map f: Bool; var b: Bool; b: Nat; eqn f = b;");
        assert!(matches!(err, WellTypedError::DuplicateEquationVariable { .. }), "{err}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_binders_bind_and_shadow() {
        // `n` is bound by the quantifier rather than free, and the lambda's
        // `b` shadows the equation variable, so neither trips the free-variable
        // check on the right-hand side.
        let system = UntypedDataSpecification::parse(
            "map f: Bool -> Bool; var b: Bool; eqn f(b) = forall n: Nat. (lambda b: Bool. b)(b == (n == n));",
        )
        .unwrap();
        check_system_specification(&UntypedDataSpecification::default(), &system)
            .unwrap_or_else(|err| panic!("binder-bound names should not be reported: {err}"));
    }
}
