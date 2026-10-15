use std::convert::Infallible;
use std::ops::ControlFlow;

use rustc_hash::FxHashMap;

use merc_syntax::ComplexSort;
use merc_syntax::ConstructorId;
use merc_syntax::DataExpr;
use merc_syntax::DataExprKind;
use merc_syntax::MapId;
use merc_syntax::SortDecl;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::SortId;
use merc_syntax::Span;
use merc_syntax::Traverse;
use merc_syntax::UntypedDataSpecification;

use crate::DataSpecification;
use crate::EquationTyping;
use crate::NameTarget;
use crate::ResolvedSort;
use crate::ResolvedSortId;
use crate::Roots;
use crate::TypeCheckContext;
use crate::for_each_expr_node;
use crate::is_system_generated_name;
use crate::resolve_sort;
use crate::unreachable_not_a_value_sort;

/// The typing of a document's data specification (or of one expression): one
/// [`TypedNode`] per checked expression node, in generation order, keyed by
/// [`Span`] rather than the crate's internal `ExprId`. `ExprId` is assigned
/// over the *lowered* expression tree, which can contain nodes with no
/// counterpart in the original `DataExpr` a caller parsed, so it can't be
/// reconstructed externally.
///
/// # Caveats
///
/// - A node synthesized during lowering inherits the span of the surface
///   expression it came from, so more than one [`TypedNode`] can share a span;
///   see [`Self::at_offset`]'s tie-break rule.
/// - [`TypedNode::sort`] is reconstructed from the resolved sort, not read from
///   a declaration, and always carries [`Span::default`] — name resolution and
///   alias normalization discard or relocate a [`SortExpression`]'s original
///   span. A sort *reference* (see [`ResolvedName::Sort`]) is unaffected: its
///   span is captured before normalization ever touches it.
/// - `TypedNode::sort` is `None` for a node with no data sort at all: an
///   action/process occurrence (its declared argument sorts are a *domain*, not
///   a value sort) or a [`ResolvedName::Sort`] occurrence (not a data
///   expression at all) — unlike every other `TypedNode`, which is always built
///   from a checked [`merc_syntax::DataExpr`].
#[derive(Debug, Default, Clone)]
pub struct TypingInfo {
    nodes: Vec<TypedNode>,
}

/// The typing of a single expression node. See [`TypingInfo`]'s doc comment for the caveats on
/// `span` and `sort`.
#[derive(Debug, Clone)]
pub struct TypedNode {
    /// The node's location in the original source.
    pub span: Span,
    /// The node's inferred sort, reconstructed as a [`SortExpression`] so it can be printed.
    pub sort: Option<SortExpression>,
    /// What this node's identifier resolved to. `None` for every node that isn't an `Id`.
    pub name: Option<ResolvedName>,
}

/// What an identifier ([`TypedNode::name`]) resolved to.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ResolvedName {
    /// An equation variable, a process/PBES/PRES parameter, or a `sum`/`dist`/quantifier binder.
    Variable {
        name: String,
        /// The declaring binder's span, carried by the `Resolved` occurrence itself.
        declaration: Option<Span>,
    },
    /// A user-declared constructor.
    Constructor {
        name: String,
        id: ConstructorId,
        /// The declaration's span, when it has one.
        declaration: Option<Span>,
    },
    /// A user-declared mapping.
    Mapping {
        name: String,
        id: MapId,
        declaration: Option<Span>,
    },
    /// A declared Appendix-B symbol (`succ`, `@c0`, …) — not a user declaration.
    SystemDefined {
        name: String,
        /// `None` when checked against a discarded `SourceMap`.
        declaration: Option<Span>,
    },
    /// A polymorphic built-in (`==`, `!=`, …), whose concrete meaning follows
    /// from the inferred argument sorts rather than one declaration.
    Builtin { name: String },
    /// A declared action.
    Action { name: String, declaration: Option<Span> },
    /// A declared process.
    Process { name: String, declaration: Option<Span> },
    /// A bare action name with no argument list to disambiguate an overload by,
    /// e.g., in hide.
    ActionSet {
        name: String,
        /// Every declaration sharing this name with a real span, in declaration order.
        declarations: Vec<Span>,
    },
    /// A PBES/PRES propositional-variable instantiation (`X(e1, e2)`), pushed at the
    /// identifier's own span rather than the whole `PropVarInst`.
    PropositionalVariable { name: String, declaration: Option<Span> },
    /// A state-formula fixpoint-variable instantiation (`X(e1, e2)` referencing an enclosing
    /// `mu X(...)`/`nu X(...)`), pushed at the identifier's own span. `declaration` is the
    /// enclosing binder's own span.
    StateVariable { name: String, declaration: Option<Span> },
    /// A sort-name reference, e.g., `D` in `map f: D -> D;`, anywhere where the
    /// user writes a sort by name.
    Sort { name: String, declaration: Option<Span> },
}

impl TypingInfo {
    /// All nodes, in generation order.
    pub fn nodes(&self) -> &[TypedNode] {
        &self.nodes
    }

    /// Consumes `self`, returning the nodes.
    pub fn into_nodes(self) -> Vec<TypedNode> {
        self.nodes
    }

    /// The most specific node whose span contains `offset`, for the usual
    /// hover/go-to-definition query.
    ///
    /// A span's end is treated as inclusive for this lookup, so a cursor
    /// sitting right after a token's last character still resolves to the
    /// token, matching the editor convention.
    ///
    /// When several nodes tie for the smallest span the last one in generation
    /// order is chosen. This can happen for synthesized nodes.
    pub fn at_offset(&self, offset: usize) -> Option<&TypedNode> {
        let mut best: Option<&TypedNode> = None;
        for node in &self.nodes {
            if node.span.start > offset || offset > node.span.end {
                continue;
            }

            let width = node.span.end - node.span.start;
            let is_narrower_or_tied = match best {
                Some(current) => width <= current.span.end - current.span.start,
                None => true,
            };

            if is_narrower_or_tied {
                best = Some(node);
            }
        }
        best
    }

    pub(crate) fn merge(&mut self, mut other: TypingInfo) {
        self.nodes.append(&mut other.nodes);
    }

    /// Records a single resolved name at `span` directly, for specification
    /// parts that are not data expressions.
    pub(crate) fn push(&mut self, span: Span, name: ResolvedName) {
        self.nodes.push(TypedNode {
            span,
            sort: None,
            name: Some(name),
        });
    }
}

/// Builds the [`TypingInfo`] of `roots` from `typing`, the already-computed sort-inference result of
/// exactly those roots: walks them in [`ExprId`](crate::ExprId) order, reading each node's span, identifier and
/// declaration off the node itself.
///
/// A synthesized node (the desugared `Id("+")` of `x + y`, a list literal's cons chain, …)
/// carries the span of the whole surface expression it was lowered from.
pub(crate) fn build(spec: &DataSpecification, roots: Roots<'_>, typing: &EquationTyping) -> TypingInfo {
    let mut info = TypingInfo::default();
    for_each_expr_node(roots.exprs(), |id, expr| {
        let name = typing.names.get(&id).map(|&target| {
            let (name, declaration) = match &expr.node {
                DataExprKind::Id(name) => (name, None),
                DataExprKind::Resolved(name, _, declaration) => (name, Some(declaration.clone())),
                _ => unreachable!("only an identifier has a name target"),
            };
            resolved_name(spec, target, name.clone(), declaration)
        });
        info.nodes.push(TypedNode {
            span: expr.span.clone(),
            sort: Some(sort_expression(
                spec.context(),
                spec.data_specification(),
                typing.sorts[*id],
            )),
            name,
        });
    });
    info
}

/// What an identifier occurrence named `name`, resolved to `target`, refers to. `declaration` is
/// the declaring binder's span for a `Resolved` variable occurrence.
fn resolved_name(
    spec: &DataSpecification,
    target: NameTarget,
    name: String,
    declaration: Option<Span>,
) -> ResolvedName {
    match target {
        NameTarget::Variable => ResolvedName::Variable { name, declaration },
        NameTarget::Builtin => ResolvedName::Builtin { name },
        NameTarget::Op { sort } => spec.declaration_index().operation(name, sort),
    }
}

/// The reverse lookup from a name occurrence to its declaration, built once at the end of
/// [`DataSpecification::from_untyped`], covering the user's and the system-defined
/// specification's constructors, mappings and sorts.
///
/// `(name, ResolvedSortId)` uniquely identifies an operation; the residual case of two literally
/// duplicated declarations (legal: `map f: Nat; f: Nat;`) resolves to the first. Keyed by name
/// first so a lookup can borrow the `&str` rather than allocate a key; the overloads of one name
/// are few, so the sort is matched linearly.
#[derive(Clone)]
pub(crate) struct DeclarationIndex {
    operations: FxHashMap<String, Vec<(ResolvedSortId, Operation)>>,
    /// The [`SortId`] of every user sort declaration, by name (first wins).
    sorts: FxHashMap<String, SortId>,
    /// The declaration span of every system-defined sort declaration, by name (first wins).
    system_sorts: FxHashMap<String, Option<Span>>,
}

/// A declared constructor or mapping, see [`DeclarationIndex`].
#[derive(Clone)]
enum Operation {
    Constructor(ConstructorId, Option<Span>),
    Mapping(MapId, Option<Span>),
    /// Declared only on the system-defined specification, whose own ids aren't meaningful to an
    /// outside caller — but whose declaration span is.
    System(Option<Span>),
}

impl DeclarationIndex {
    /// Requires every user constructor/mapping sort to be resolved in `ctx`.
    pub(crate) fn build(
        ctx: &mut TypeCheckContext,
        spec: &UntypedDataSpecification,
        system: &UntypedDataSpecification,
    ) -> Self {
        let mut index = DeclarationIndex {
            operations: FxHashMap::default(),
            sorts: FxHashMap::default(),
            system_sorts: FxHashMap::default(),
        };

        // Inserted user constructors first, then user mappings, then the system-defined ones, so
        // the first one wins in that order.
        for decl in &spec.constructor_declarations {
            let id = decl.id.expect("assign_declaration_ids ran during from_untyped");
            let sort = ctx.sort_of_constructor[&id];
            let operation = Operation::Constructor(id, declared_span(&decl.identifier.span));
            index.insert_operation(&decl.identifier, sort, operation);
        }

        for decl in &spec.map_declarations {
            let id = decl.id.expect("assign_declaration_ids ran during from_untyped");
            let sort = ctx.sort_of_map[&id];
            let operation = Operation::Mapping(id, declared_span(&decl.identifier.span));
            index.insert_operation(&decl.identifier, sort, operation);
        }

        let system_operations = system
            .constructor_declarations
            .iter()
            .map(|decl| (&decl.identifier, &decl.sort))
            .chain(
                system
                    .map_declarations
                    .iter()
                    .map(|decl| (&decl.identifier, &decl.sort)),
            );
        for (identifier, sort) in system_operations {
            let sort = resolve_sort(ctx, spec, sort);
            index.insert_operation(identifier, sort, Operation::System(Some(identifier.span.clone())));
        }

        for (i, decl) in spec.sort_declarations.iter().enumerate() {
            index.sorts.entry(decl.identifier.clone()).or_insert(SortId::new(i));
        }

        for decl in &system.sort_declarations {
            index
                .system_sorts
                .entry(decl.identifier.clone())
                .or_insert_with(|| declared_span(&decl.span));
        }

        index
    }

    /// Records `operation` unless `(name, sort)` is already present (first one wins).
    fn insert_operation(&mut self, name: &str, sort: ResolvedSortId, operation: Operation) {
        let overloads = self.operations.entry(name.to_string()).or_default();
        if !overloads.iter().any(|(existing, _)| *existing == sort) {
            overloads.push((sort, operation));
        }
    }

    /// What the constructor or mapping `name` with overload `sort` resolves to.
    fn operation(&self, name: String, sort: ResolvedSortId) -> ResolvedName {
        let operation = self
            .operations
            .get(&name)
            .and_then(|overloads| overloads.iter().find(|(candidate, _)| *candidate == sort))
            .map(|(_, operation)| operation.clone());

        match operation {
            Some(Operation::Constructor(id, declaration)) => ResolvedName::Constructor { name, id, declaration },
            Some(Operation::Mapping(id, declaration)) => ResolvedName::Mapping { name, id, declaration },
            Some(Operation::System(declaration)) => ResolvedName::SystemDefined { name, declaration },
            None => ResolvedName::SystemDefined {
                name,
                declaration: None,
            },
        }
    }
}

/// `Span::default()` marks a declaration synthesized without a real source location, rather than a
/// real position at the start of the file; normalize it to `None` so a consumer doesn't render a
/// misleading declaration site.
pub(crate) fn declared_span(span: &Span) -> Option<Span> {
    (*span != Span::default()).then(|| span.clone())
}

/// A single sort-name occurrence gathered by [`collect_sort_name_references`].
pub(crate) struct SortReference {
    pub(crate) span: Span,
    pub(crate) name: String,
    pub(crate) complex: Option<ComplexSort>,
    /// The occurrence's own [`SortId`].
    pub(crate) id: Option<SortId>,
}

/// Gathers the raw material [`push_sort_references`] turns into [`ResolvedName::Sort`] nodes.
/// Occurrences inside the data specification's own declarations must be collected before
/// [`crate::normalize_sorts`] rewrites the tree in place — an alias reference is replaced by its
/// own expansion, discarding both the reference's span and its name (see [`TypingInfo`]'s doc
/// comment on why a resolved sort's span can't be trusted). Occurrences inside a process/PBES/PRES
/// specification live in a syntax tree that pass never touches, so they can be gathered any time
/// after parsing instead.
///
/// Appends every `Reference`/`Resolved`/`Simple`/`Complex` leaf reachable in `sort` to `out`, as
/// `(its own occurrence span, name)`. Other compound kinds (`Product`, `Function`,
/// `FlattenedFunction`, `Struct`) are walked via [`Traverse`] until a named leaf is reached.
pub(crate) fn collect_sort_name_references(sort: &SortExpression, out: &mut Vec<SortReference>) {
    sort.visit::<Infallible, _>(|node| {
        match &node.node {
            SortExpressionKind::Reference(name) => {
                out.push(SortReference {
                    span: node.span.clone(),
                    name: name.clone(),
                    complex: None,
                    id: None,
                });
            }
            SortExpressionKind::Resolved(name, id) => {
                out.push(SortReference {
                    span: node.span.clone(),
                    name: name.clone(),
                    complex: None,
                    id: Some(*id),
                });
            }
            SortExpressionKind::Simple(sort) => {
                out.push(SortReference {
                    span: node.span.clone(),
                    name: sort.to_string(),
                    complex: None,
                    id: None,
                });
            }
            SortExpressionKind::Complex(complex_sort, _) => {
                let keyword = complex_sort.to_string();
                let span = Span {
                    start: node.span.start,
                    end: node.span.start + keyword.len(),
                };
                out.push(SortReference {
                    span,
                    name: keyword,
                    complex: Some(*complex_sort),
                    id: None,
                });
            }
            _ => {}
        }
        ControlFlow::Continue(())
    });
}

/// Every sort-name reference reachable in `spec`'s own declarations.
///
/// Must be called before [`crate::normalize_sorts`].
pub(crate) fn collect_data_specification_sort_references(spec: &UntypedDataSpecification) -> Vec<SortReference> {
    let mut out = Vec::new();

    for expr in spec.sort_declarations.iter().filter_map(|decl| decl.expr.as_ref()) {
        collect_sort_name_references(expr, &mut out);
    }

    for decl in &spec.constructor_declarations {
        collect_sort_name_references(&decl.sort, &mut out);
    }

    for decl in &spec.map_declarations {
        collect_sort_name_references(&decl.sort, &mut out);
    }

    for eqn_spec in &spec.equation_declarations {
        for var in &eqn_spec.variables {
            collect_sort_name_references(&var.sort, &mut out);
        }

        for eqn in &eqn_spec.equations {
            collect_data_expr_sort_references(&eqn.lhs, &mut out);
            collect_data_expr_sort_references(&eqn.rhs, &mut out);
            if let Some(condition) = &eqn.condition {
                collect_data_expr_sort_references(condition, &mut out);
            }
        }
    }

    out
}

/// Every `lambda`/quantifier/comprehension binder's declared sort inside `expr`, appended to
/// `out`. `Traverse` recurses into `expr`'s own `DataExpr` children for free; only the binder's
/// own `sort` (an `IdDecl`, a different node type) needs handling at each matching node.
fn collect_data_expr_sort_references(expr: &DataExpr, out: &mut Vec<SortReference>) {
    expr.visit::<Infallible, _>(|node| {
        match &node.node {
            DataExprKind::Lambda { variables, .. } | DataExprKind::Quantifier { variables, .. } => {
                for var in variables {
                    collect_sort_name_references(&var.sort, out);
                }
            }
            DataExprKind::SetBagComp { variable, .. } => collect_sort_name_references(&variable.sort, out),
            _ => {}
        }
        ControlFlow::Continue(())
    });
}

/// `reference`'s own [`SortId`] declaration, and whether it names a user sort or a system-internal
/// one. Returns the whole declaration rather than just its name, since [`push_sort_references`]
/// needs the declaration's span.
fn sort_declaration_by_id(spec: &UntypedDataSpecification, id: SortId) -> Option<(&SortDecl, bool)> {
    let decl = spec.sort_declarations.get(*id)?;
    Some((decl, is_system_generated_name(&decl.identifier)))
}

/// Resolves each occurrence in `references` to its declaration and pushes
/// [`ResolvedName::Sort`]/[`ResolvedName::SystemDefined`] into `typing`.
///
/// An occurrence with a real [`SortId`] resolves via [`sort_declaration_by_id`], either from its
/// own [`SortReference::id`] or, for an unresolved [`SortExpressionKind::Reference`], by name via
/// the [`DeclarationIndex`]. Two shapes carry no [`SortId`]: a container-sort keyword (`List`, `Set`, …),
/// which has no declaration site, and a primitive basic-sort name (`Bool`, `Nat`, …), which still
/// has a real declaration span in `system`'s own textual re-declaration of it (`sort Nat;` in
/// `nat.mcrl2`), found by name instead.
pub(crate) fn push_sort_references(spec: &DataSpecification, references: &[SortReference], typing: &mut TypingInfo) {
    let index = spec.declaration_index();
    for reference in references {
        let name = &reference.name;
        let id = reference.id.or_else(|| index.sorts.get(name).copied());

        if let Some(id) = id
            && let Some((decl, is_system)) = sort_declaration_by_id(spec.data_specification(), id)
        {
            let declaration = declared_span(&decl.span);
            typing.push(
                reference.span.clone(),
                if is_system {
                    ResolvedName::SystemDefined {
                        name: name.clone(),
                        declaration,
                    }
                } else {
                    ResolvedName::Sort {
                        name: name.clone(),
                        declaration,
                    }
                },
            );
        } else if reference.complex.is_some() {
            // A container-sort keyword (`List`, `Set`, …) never has a `sort` declaration of its
            // own — see `SortReference`'s doc comment.
            typing.push(
                reference.span.clone(),
                ResolvedName::SystemDefined {
                    name: name.clone(),
                    declaration: None,
                },
            );
        } else if let Some(declaration) = index.system_sorts.get(name) {
            typing.push(
                reference.span.clone(),
                ResolvedName::SystemDefined {
                    name: name.clone(),
                    declaration: declaration.clone(),
                },
            );
        }
    }
}

/// Records a binder's own declaration occurrence.
pub(crate) fn push_binder_declaration(
    data: &DataSpecification,
    typing: &mut TypingInfo,
    span: Span,
    name: String,
    sort: ResolvedSortId,
) {
    let sort = sort_expression(data.context(), data.data_specification(), sort);
    typing.nodes.push(TypedNode {
        span: span.clone(),
        sort: Some(sort),
        name: Some(ResolvedName::Variable {
            name,
            declaration: Some(span),
        }),
    });
}

/// Rebuilds `id` as a [`SortExpression`], so it can be displayed via its existing
/// [`std::fmt::Display`] impl and so a `Def` sort carries a `SortId` a consumer can use for sort
/// go-to-definition. Mirrors [`crate::lower_sort`]'s structural recursion (same crate, targeting
/// the binary aterm format instead of the AST's own sort type).
///
/// Every produced node gets [`Span::default`].
fn sort_expression(ctx: &TypeCheckContext, spec: &UntypedDataSpecification, id: ResolvedSortId) -> SortExpression {
    match ctx.sorts.get(id) {
        ResolvedSort::Unit => unreachable_not_a_value_sort("Unit"),
        ResolvedSort::Primitive(sort) => SortExpressionKind::Simple(*sort).into(),
        ResolvedSort::Container { op, subsort } => {
            SortExpressionKind::Complex(*op, Box::new(sort_expression(ctx, spec, *subsort))).into()
        }
        ResolvedSort::Function { domain, range } => SortExpressionKind::FlattenedFunction {
            domain: domain.iter().map(|&sort| sort_expression(ctx, spec, sort)).collect(),
            range: Box::new(sort_expression(ctx, spec, *range)),
        }
        .into(),
        ResolvedSort::Def(def) => {
            let name = ctx.sort_display_name(spec, *def).into_owned();
            SortExpressionKind::Resolved(name, *def).into()
        }
        ResolvedSort::TypeVar(_) => unreachable_not_a_value_sort("Var"),
    }
}
