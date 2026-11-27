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
