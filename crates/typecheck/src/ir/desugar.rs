use std::convert::Infallible;

use log::debug;
use log::trace;

use merc_syntax::ConstructorDecl;
use merc_syntax::ConstructorId;
use merc_syntax::DataExpr;
use merc_syntax::DataExprKind;
use merc_syntax::IdDecl;
use merc_syntax::MapId;
use merc_syntax::Sort;
use merc_syntax::SortDecl;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::Span;
use merc_syntax::Spanned;
use merc_syntax::Traverse;
use merc_syntax::UntypedDataSpecification;

/// Hoists every anonymous structured sort (a `struct` occurring inside another
/// sort expression rather than as the body of a sort declaration) into a fresh
/// `@struct<n>` sort declaration, replacing the occurrence by a reference to
/// it.
///
/// Structurally identical structs denote the same sort in mCRL2, so identical
/// occurrences share one declaration, and an anonymous struct that matches an
/// already-seen named struct alias reuses the user's name.
///
/// Runs before name resolution so the generated declarations are resolved and
/// checked exactly like user-written ones.
pub(crate) fn hoist_anonymous_structs(spec: &mut UntypedDataSpecification) {
    let mut hoister = Hoister {
        table: Vec::new(),
        fresh: Vec::new(),
    };

    for declaration in &mut spec.sort_declarations {
        match &mut declaration.expr {
            // A top-level struct is the named struct itself and stays; only
            // structs nested inside its constructor arguments are hoisted.
            Some(Spanned {
                node: SortExpressionKind::Struct { inner },
                ..
            }) => {
                for constructor in inner.iter_mut() {
                    for (_, sort) in &mut constructor.args {
                        *sort = hoister.hoist(sort.clone());
                    }
                }
                hoister.table.push((
                    SortExpressionKind::Struct { inner: inner.clone() }.into(),
                    declaration.identifier.clone(),
                ));
            }

            // Non-struct sort alias (e.g. `sort A = List(struct t);`): the
            // anonymous struct occurs inside a sort *declaration*, so it
            // should still generate its constructors like any other
            // declaration-position occurrence.
            Some(expr) => *expr = hoister.hoist(expr.clone()),
            None => {}
        }
    }

    for constructor in &mut spec.constructor_declarations {
        constructor.sort = hoister.hoist_non_decl(constructor.sort.clone());
    }

    for map in &mut spec.map_declarations {
        map.sort = hoister.hoist_non_decl(map.sort.clone());
    }

    for equation in &mut spec.equation_declarations {
        for variable in &mut equation.variables {
            variable.sort = hoister.hoist_non_decl(variable.sort.clone());
        }

        for eqn in &mut equation.equations {
            if let Some(condition) = &mut eqn.condition {
                hoist_binder_sorts_in_place(&mut hoister, condition);
            }

            hoist_binder_sorts_in_place(&mut hoister, &mut eqn.lhs);
            hoist_binder_sorts_in_place(&mut hoister, &mut eqn.rhs);
        }
    }

    spec.sort_declarations.append(&mut hoister.fresh);
}

/// Hoists the anonymous structs on every `expr` in place.
fn hoist_binder_sorts_in_place(hoister: &mut Hoister, expr: &mut DataExpr) {
    expr.transform(|expr| match &mut expr.node {
        DataExprKind::SetBagComp { variable, predicate: _ } => {
            variable.sort = hoister.hoist_non_decl(variable.sort.clone());
        }
        DataExprKind::Lambda { variables, body: _ }
        | DataExprKind::Quantifier {
            op: _,
            variables,
            body: _,
        } => {
            for variable in variables {
                variable.sort = hoister.hoist_non_decl(variable.sort.clone());
            }
        }
        _ => {}
    });
}

struct Hoister {
    /// Struct bodies that are already available under a name, so structurally
    /// identical occurrences resolve to the same sort.
    table: Vec<(SortExpression, String)>,
    /// The generated `@struct<n>` declarations.
    fresh: Vec<SortDecl>,
}

impl Hoister {
    /// Replaces every anonymous struct in `sort` by a reference to its named
    /// declaration. The named declaration retains the struct *body*, so
    /// [`desugar_structured_sorts`] will generate its constructors, recognisers
    /// and projections. Use only for structs nested inside a named sort
    /// declaration's constructor arguments.
    fn hoist(&mut self, sort: SortExpression) -> SortExpression {
        sort.apply(|expr| -> Result<Option<SortExpression>, Infallible> {
            if let SortExpressionKind::Struct { inner } = &expr.node {
                // Hoist the constructor arguments first, so identical structs
                // have identical bodies regardless of nesting.
                let mut inner = inner.clone();
                for constructor in &mut inner {
                    for (_, sort) in &mut constructor.args {
                        *sort = self.hoist(sort.clone());
                    }
                }

                return Ok(Some(
                    SortExpressionKind::Reference(self.name_for(SortExpressionKind::Struct { inner }.into())).into(),
                ));
            }

            Ok(None)
        })
        .expect("The inner function never fails")
    }

    /// Like `hoist` but generates an body-less declaration for any anonymous
    /// struct not already registered from a declaration-position `sort X =
    /// struct …;`.
    ///
    /// # Example
    ///
    /// ```text
    /// sort D = struct c | d;
    /// map f: struct c | d -> Bool;
    /// ```
    ///
    /// Here `f`'s domain has the same body as `D`, so it is hoisted to a
    /// reference to `D` itself and `f` can be applied to `c`/`d`. Written in
    /// isolation, without a matching declaration-position `sort D = ...;`,
    /// the same `struct c | d` in `f`'s domain would hoist to a fresh,
    /// body-less `@struct<n>` sort — `c` and `d` are never registered as
    /// constructors, so `f` would have no value it could ever be applied to.
    fn hoist_non_decl(&mut self, sort: SortExpression) -> SortExpression {
        sort.apply(|expr| -> Result<Option<SortExpression>, Infallible> {
            if let SortExpressionKind::Struct { inner } = &expr.node {
                let mut inner = inner.clone();
                for constructor in &mut inner {
                    for (_, sort) in &mut constructor.args {
                        *sort = self.hoist_non_decl(sort.clone());
                    }
                }
                return Ok(Some(
                    SortExpressionKind::Reference(self.name_for_non_decl(SortExpressionKind::Struct { inner }.into()))
                        .into(),
                ));
            }
            Ok(None)
        })
        .expect("inner never fails")
    }

    /// The name declaring `body`, generating a fresh `@struct<n>` declaration
    /// WITH a body when it has not been seen before.
    fn name_for(&mut self, body: SortExpression) -> String {
        if let Some((_, name)) = self.table.iter().find(|(existing, _)| *existing == body) {
            let name = name.clone();
            // Upgrade an existing abstract declaration to one with a body.
            if let Some(decl) = self.fresh.iter_mut().find(|d| d.identifier == name && d.expr.is_none()) {
                debug!("desugar: upgraded abstract struct '{name}' to full declaration");
                decl.expr = Some(body);
            }
            return name;
        }

        let name = format!("@struct{}", self.fresh.len());
        debug!("desugar: hoisted anonymous struct '{body}' as sort '{name}'");
        self.table.push((body.clone(), name.clone()));
        self.fresh.push(SortDecl {
            identifier: name.clone(),
            expr: Some(body),
            span: Span::default(),
            id: None,
        });
        name
    }

    /// The name for `body` in a **non-declaration** context.  If `body` was
    /// already registered (from a prior declaration-position occurrence), that
    /// name is returned unchanged.  Otherwise a fresh `@struct<n>` with *no
    /// body* is registered: `desugar_structured_sorts` will skip it and no
    /// constructors are generated.
    fn name_for_non_decl(&mut self, body: SortExpression) -> String {
        if let Some((_, name)) = self.table.iter().find(|(existing, _)| *existing == body) {
            return name.clone();
        }
        let name = format!("@struct{}", self.fresh.len());
        trace!("desugar: hoisted anonymous struct '{body}' as abstract sort '{name}' (non-decl position)");
        self.table.push((body, name.clone()));
        // No body → desugar_structured_sorts skips constructor generation.
        self.fresh.push(SortDecl {
            identifier: name.clone(),
            expr: None,
            span: Span::default(),
            id: None,
        });
        name
    }
}

/// Desugars every named structured-sort declaration into an abstract sort plus
/// the constructors, recognisers and projections it introduces.
///
/// `sort D = struct c1(p: A)?is_c1 | c2;` becomes
///
/// ```text
/// sort D;
/// cons c1: A -> D;  c2: D;
/// map  is_c1: D -> Bool;   % only for constructors with a recogniser
///      p: D -> A;          % only for named projections (deduplicated)
/// ```
///
/// Returns the constructor list of every desugared structured sort, from which
/// `structured_sort_equations` generates the defining equations (Appendix
/// B.10) for the system-defined specification.
///
/// Runs after name resolution, so the generated sorts are already resolved and
/// flattened, and the structured sort keeps its `SortId`.
pub(crate) fn desugar_structured_sorts(spec: &mut UntypedDataSpecification) -> Vec<Vec<ConstructorDecl>> {
    let mut constructors: Vec<IdDecl<ConstructorId>> = Vec::new();
    let mut mappings: Vec<IdDecl<MapId>> = Vec::new();
    let mut structs = Vec::new();

    for declaration in &mut spec.sort_declarations {
        let inner = match &declaration.expr {
            Some(Spanned {
                node: SortExpressionKind::Struct { inner },
                ..
            }) => inner.clone(),
            _ => continue,
        };

        let id = declaration.id.expect("Name must have been resolved");
        let sort: SortExpression = SortExpressionKind::Resolved(declaration.identifier.clone(), id).into();
        // The structured sort becomes an abstract sort carrying its constructors.
        declaration.expr = None;
        debug!(
            "desugar: struct '{}' desugared into {} constructor(s)",
            declaration.identifier,
            inner.len()
        );

        for constructor in &inner {
            // cons c: A_1 # ... # A_n -> D  (or c: D when it has no arguments).
            let domain = constructor.args.iter().map(|(_, sort)| sort.clone()).collect();
            let constructor_sort = function_sort(domain, sort.clone());
            trace!("desugar:   cons {}: {constructor_sort}", constructor.name.node);
            constructors.push(IdDecl::new(
                constructor.name.node.clone(),
                constructor_sort,
                constructor.name.span.clone(),
            ));

            // map is_c: D -> Bool  (recogniser), when one is declared.
            if let Some(recogniser) = &constructor.recogniser {
                let recogniser_sort = function_sort(vec![sort.clone()], SortExpressionKind::Simple(Sort::Bool).into());
                push_unique(
                    &mut mappings,
                    IdDecl::new(recogniser.node.clone(), recogniser_sort, recogniser.span.clone()),
                );
            }

            // map p: D -> A  (projection), when a name is declared for the argument.
            for (projection, argument_sort) in &constructor.args {
                if let Some(projection) = projection {
                    let projection_sort = function_sort(vec![sort.clone()], argument_sort.clone());
                    push_unique(
                        &mut mappings,
                        IdDecl::new(projection.node.clone(), projection_sort, projection.span.clone()),
                    );
                }
            }
        }

        structs.push(inner);
    }

    spec.constructor_declarations.append(&mut constructors);
    spec.map_declarations.append(&mut mappings);

    structs
}

/// Builds `domain_0 # ... # domain_n -> range` as an already-flattened function
/// sort, or just `range` when there are no arguments.
fn function_sort(domain: Vec<SortExpression>, range: SortExpression) -> SortExpression {
    if domain.is_empty() {
        range
    } else {
        SortExpressionKind::FlattenedFunction {
            domain,
            range: Box::new(range),
        }
        .into()
    }
}

/// Appends `mapping` unless a declaration with the same name and sort is already present, so a
/// projection shared by several constructors is generated only once, at the *first* constructor's
/// own span.
fn push_unique(mappings: &mut Vec<IdDecl<MapId>>, mapping: IdDecl<MapId>) {
    if !mappings
        .iter()
        .any(|existing| existing.identifier == mapping.identifier && existing.sort == mapping.sort)
    {
        trace!("desugar:   map {}: {}", mapping.identifier, mapping.sort);
        mappings.push(mapping);
    }
}

#[cfg(test)]
mod tests {
    use merc_syntax::UntypedDataSpecification;

    use crate::DataSpecification;

    /// Returns the constructor and mapping names of the type-checked spec.
    fn constructors_and_mappings(text: &str) -> (Vec<String>, Vec<String>) {
        let checked = DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()).unwrap();
        let spec = checked.data_specification();
        let constructors = spec
            .constructor_declarations
            .iter()
            .map(|declaration| declaration.identifier.node.clone())
            .collect();
        let mappings = spec
            .map_declarations
            .iter()
            .map(|declaration| declaration.identifier.node.clone())
            .collect();
        (constructors, mappings)
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow
    fn test_struct_desugars_to_constructors() {
        let (constructors, mappings) = constructors_and_mappings("sort D = struct c1(p1: Bool)?is_c1 | c2;");
        assert!(constructors.contains(&"c1".to_string()));
        assert!(constructors.contains(&"c2".to_string()));
        assert!(mappings.contains(&"is_c1".to_string()));
        assert!(mappings.contains(&"p1".to_string()));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow
    fn test_reused_projection_is_generated_once() {
        // `p` is shared by `c` and `d` with the same sort, so only one mapping
        // is generated for it.
        let (_, mappings) = constructors_and_mappings("sort S = struct c(p: Bool) | d(p: Bool, q: S);");
        assert_eq!(mappings.iter().filter(|name| *name == "p").count(), 1);
        assert!(mappings.contains(&"q".to_string()));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow
    fn test_struct_equations_are_in_system_spec() {
        let checked = DataSpecification::from_untyped(
            UntypedDataSpecification::parse("sort D = struct c1(p1: Bool)?is_c1 | c2;").unwrap(),
        )
        .unwrap();

        let equations: Vec<String> = checked
            .system_defined_specification()
            .equation_declarations
            .iter()
            .flat_map(|eqn_spec| &eqn_spec.equations)
            .map(|eqn| format!("{} = {}", eqn.lhs, eqn.rhs))
            .collect();

        // The recogniser, projection and comparison equations are generated
        // (operators are lowered to named applications by then).
        assert!(equations.iter().any(|eqn| eqn.starts_with("is_c1(c1(")));
        assert!(equations.iter().any(|eqn| eqn.starts_with("p1(c1(")));
        assert!(equations.iter().any(|eqn| eqn.contains("==(c2, c2)")));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow
    fn test_anonymous_struct_in_mapping_is_desugared() {
        // An anonymous struct in a mapping declaration is hoisted to a fresh
        // **abstract** sort (no body, no constructors): mCRL2 only generates
        // constructors for structs in sort-declaration position.
        let (constructors, _) = constructors_and_mappings("map f: struct c | d;");
        assert!(
            !constructors.contains(&"c".to_string()),
            "c must not be a constructor from a map-position struct"
        );
        assert!(
            !constructors.contains(&"d".to_string()),
            "d must not be a constructor from a map-position struct"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow
    fn test_nested_anonymous_struct_is_desugared() {
        // The struct nested inside `t`'s argument is hoisted and desugared, so
        // its constructor `e` is declared too.
        let (constructors, _) = constructors_and_mappings("sort S = struct t(struct e(Nat));");
        assert!(constructors.contains(&"t".to_string()));
        assert!(constructors.contains(&"e".to_string()));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_identical_anonymous_structs_share_a_declaration() {
        // Structurally identical structs in map positions share one abstract
        // sort declaration (deduplication still works), but generate no
        // constructors (non-decl position).
        let (constructors, _) = constructors_and_mappings("map f: struct c;\n    g: struct c;");
        assert_eq!(
            constructors.iter().filter(|name| *name == "c").count(),
            0,
            "c must not be a constructor from map-position structs"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_anonymous_struct_reuses_named_alias() {
        // An anonymous struct that matches a named struct alias is the same
        // sort as the alias, so no second declaration (and constructor) is
        // generated.
        let (constructors, _) = constructors_and_mappings("sort D = struct c;\nmap f: struct c;");
        assert_eq!(constructors.iter().filter(|name| *name == "c").count(), 1);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under mir i
    fn test_recursive_struct_is_non_empty() {
        // A recursive structured sort with a base constructor is non-empty and
        // type checks after desugaring.
        DataSpecification::from_untyped(
            UntypedDataSpecification::parse("sort Tree = struct leaf | node(Tree, Tree);").unwrap(),
        )
        .expect("a recursive struct with a base case is non-empty");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow
    fn test_struct_over_abstract_arguments_is_non_empty() {
        // A struct whose constructors take abstract-sort arguments is non-empty
        // (the abstract arguments are assumed non-empty).
        DataSpecification::from_untyped(
            UntypedDataSpecification::parse("sort A;\n     B;\nsort S = struct c(A) | d(B);").unwrap(),
        )
        .expect("a struct over abstract argument sorts is non-empty");
    }
}

#[cfg(test)]
mod stack_depth_probe {
    //! `hoist_binder_sorts_in_place` is one of the three `crates/typecheck` call sites
    //! `review/stack-overflow-recursion.md` named as depending on `Traverse::transform`'s own
    //! stack safety without having a regression test of its own; this is that test. The tree here
    //! is built directly, bypassing the parser (which has its own, unrelated recursion limit).
    use merc_syntax::DataExprKind;
    use merc_syntax::DataExprUnaryOp;
    use merc_syntax::Span;

    use super::Hoister;
    use super::hoist_binder_sorts_in_place;

    #[test]
    fn deeply_nested_unary_does_not_overflow_the_stack() {
        let mut hoister = Hoister {
            table: Vec::new(),
            fresh: Vec::new(),
        };
        let mut expr = DataExprKind::Bool(true).spanned(Span::default());
        for _ in 0..100_000 {
            expr = DataExprKind::Unary {
                op: DataExprUnaryOp::Negation,
                expr: Box::new(expr),
            }
            .spanned(Span::default());
        }

        hoist_binder_sorts_in_place(&mut hoister, &mut expr);
    }
}
