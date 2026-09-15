use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use log::debug;
use log::trace;

use merc_syntax::ComplexSort;
use merc_syntax::DataExpr;
use merc_syntax::DataExprKind;
use merc_syntax::EqnSpec;
use merc_syntax::EqnSpecId;
use merc_syntax::EquationId;
use merc_syntax::IdDecl;
use merc_syntax::Sort;
use merc_syntax::SortExpression;
use merc_syntax::SourceMap;
use merc_syntax::Span;
use merc_syntax::TypeVarId;
use merc_syntax::UntypedDataSpecification;
use merc_syntax::VarId;
use merc_utilities::TagIndex;

use crate::DisplaySortContext;
use crate::InferSort;
use crate::InferSortId;
use crate::PolySortScheme;
use crate::ResolvedSort;
use crate::ResolvedSortId;
use crate::Signature;
use crate::SortInterner;
use crate::TemplateInstantiation;
use crate::TypeCheckContext;
use crate::Unifier;
use crate::build_builtin_scheme_signature;
use crate::is_lowered;
use crate::is_supported_binder_sort;
use crate::number_generality;
use crate::query_sort_of_equation_var;
use crate::resolve_sort;

/// A unique type for expression nodes within a single equation.
pub(crate) struct ExprTag;

/// Identifies an expression node of one equation.
///
/// Ids are assigned parents before children, and within an application the
/// arguments before the applied function (so the solver sees argument
/// constraints before the callee's overload disjunction), over the condition,
/// left-hand side and right-hand side in that order. Container literals number
/// their members in syntactic order (a bag member before its multiplicity); a
/// comprehension numbers only its predicate, and a `lambda`/`forall`/`exists`
/// only its body — the bound variables have no id, like the equation
/// variables. A `whr` numbers each assignment's right-hand side, in binding
/// order, before the body. Phase-4 lowering re-walks the same lowered AST, so
/// this numbering must stay deterministic.
pub(crate) type ExprId = TagIndex<usize, ExprTag>;

/// What a name (`Id` node) in an equation resolved to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NameTarget {
    /// An equation variable of the enclosing `var` block.
    Variable,
    /// A declared constructor or mapping (user or system-defined) with the
    /// given overload sort.
    Op { sort: ResolvedSortId },
    /// A polymorphic built-in (`==`, `!=`, `<`, `<=`, `>`, `>=`, `if`), whose
    /// concrete sort follows from the inferred argument sorts.
    Builtin,
}

/// The Phase-3 typing result of a single equation. Every equation that type
/// checks is fully inferred: a binder over a sort inference cannot model (a
/// bare product sort, which is not a valid variable sort) is now a hard error
/// rather than a silently untyped equation.
#[derive(Debug)]
pub(crate) struct EquationTyping {
    /// The inferred sort of every expression node, indexed by [ExprId].
    pub(crate) sorts: Vec<ResolvedSortId>,
    /// The source span of every expression node, parallel to `sorts`. Only
    /// filled for [EquationRole::User].
    ///
    /// A synthesized node (the desugared `Id("+")` of `x + y`, a list
    /// literal's cons chain, …) inherits the span of the whole surface
    /// expression it was lowered from, since it has no span of its own in the
    /// original source.
    pub(crate) spans: Vec<Span>,
    /// The resolution of every name, keyed by the [ExprId] of its `Id` node.
    pub(crate) names: HashMap<ExprId, NameTarget>,
    /// The identifier text of every `Id` node, keyed the same way as `names`.
    /// Only filled for [EquationRole::User], like `spans`.
    pub(crate) identifier_names: HashMap<ExprId, String>,
    /// The declaration [VarId] of every `Resolved` node (a variable reference that names its own
    /// binder), keyed the same way as `names`. Only filled for [EquationRole::User], like
    /// `spans`. Absent for a plain `Id` node resolving to [NameTarget::Variable] — e.g. an
    /// occurrence the upstream variable-resolution pass left unresolved because it names no
    /// binder in scope (rejected separately by [`NameTarget::Variable`]'s own lookup below).
    pub(crate) declarations: HashMap<ExprId, VarId>,
    /// Every visited node's own [ExprId], keyed by that node's address.
    pub(crate) node_ids: HashMap<usize, ExprId>,
}

/// The errors of Phase-3 sort inference. `Clone` so a failure can be stored in
/// the query cache and reported again on later lookups.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum InferenceError {
    #[error("the name '{name}' is not declared")]
    UndeclaredName { name: String, span: Span },

    #[error("'{expr}' is applied to arguments, but cannot have a function sort")]
    NotAFunction { expr: String, span: Span },

    #[error("the condition '{condition}' cannot have sort Bool")]
    ConditionNotBool { condition: String, span: Span },

    #[error("the body '{body}' of a forall/exists must have sort Bool")]
    QuantifierNotBool { body: String, span: Span },

    #[error(
        "'{expression}' has no valid sort assignment{}",
        sort.as_deref().map_or(String::new(), |sort| format!(" matching sort '{sort}'"))
    )]
    NoTyping {
        expression: String,
        /// The sort the expression was checked against, when inference ran with an
        /// externally-supplied expected sort ([`Roots::ExpressionAgainst`], e.g. via
        /// `check_expression_against`); `None` when checking a whole equation, where no single
        /// sort is being blamed.
        sort: Option<String>,
        span: Span,
    },

    #[error("the sorts in '{expression}' are ambiguous")]
    AmbiguousExpression { expression: String, span: Span },

    #[error("the sorts in '{expression}' are underdetermined")]
    UnderdeterminedSort { expression: String, span: Span },

    #[error("the binder sort '{sort}' in '{expression}' is not a valid variable sort")]
    InvalidBinderSort {
        sort: String,
        expression: String,
        span: Span,
    },
}

impl InferenceError {
    /// The span of the offending sub-expression (or, for `NoTyping` /
    /// `AmbiguousExpression` / `UnderdeterminedSort`, the whole equation, since
    /// no narrower sub-expression can be blamed for those). Pair with
    /// [Span::render] to show a source snippet alongside the message.
    pub fn span(&self) -> &Span {
        match self {
            InferenceError::UndeclaredName { span, .. }
            | InferenceError::NotAFunction { span, .. }
            | InferenceError::ConditionNotBool { span, .. }
            | InferenceError::QuantifierNotBool { span, .. }
            | InferenceError::NoTyping { span, .. }
            | InferenceError::AmbiguousExpression { span, .. }
            | InferenceError::UnderdeterminedSort { span, .. }
            | InferenceError::InvalidBinderSort { span, .. } => span,
        }
    }

    /// Renders this error's message, followed by a caret-annotated source
    /// snippet (see [Span::render]). `sources` must contain the original text
    /// the error was raised against — the specification for an equation
    /// error, the expression text for one raised by
    /// [`crate::DataSpecification::typecheck_expression`].
    pub fn render(&self, sources: &SourceMap) -> String {
        format!("{self}\n{}", self.span().render(sources))
    }
}

/// Which specification's equations are being checked. All roles share the
/// same [ConstraintGenerator] and [Solver]; only where a name and a
/// binder/equation-variable sort resolve from differs.
#[derive(Clone, Copy)]
enum EquationRole {
    /// Names resolve against `ctx.signature` (which already pools the system-defined basic-sort
    /// operators and the polymorphic schemes alongside the user's own declarations), then
    /// `ctx.basics_signature` again as a redundant (harmless — see `push_signature_disjuncts`)
    /// fallback; sorts via `resolve_sort`.
    User,
    /// Names resolve against `ctx.struct_signature_overrides`'s entry for this equation's own
    /// block when present, falling back to `ctx.basics_signature` and the narrow builtin-scheme
    /// table otherwise — deliberately never the full `ctx.signature`, which would leak every
    /// *other* user declaration (including an unrelated struct's same-named symbol) into a
    /// struct's own isolated equations.
    System,
    /// Checking one Appendix-B container/function-update template's own,
    /// un-instantiated equations, once, with its `type_var`-declared sort(s)
    /// held rigid.
    Template,
}

/// Returns the typing of one user equation, keyed by the id of its enclosing
/// equation specification block and its own id within that block (assigned by
/// [assign_declaration_ids](crate::assign_declaration_ids)). Memoized on
/// [TypeCheckContext::equation_typing].
pub(crate) fn query_equation_typing(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    system: &UntypedDataSpecification,
    key: (EqnSpecId, EquationId),
) -> Result<Arc<EquationTyping>, InferenceError> {
    let (eqn_spec_id, equation_id) = key;

    debug_assert!(
        spec.equation_declarations
            .get(*eqn_spec_id)
            .is_some_and(|eqn_spec| *equation_id < eqn_spec.equations.len()),
        "equation typing key {key:?} must index an equation of the specification"
    );

    ctx.get_or_compute(
        |ctx| &mut ctx.equation_typing,
        key,
        |ctx| infer_equation(ctx, spec, system, EquationRole::User, eqn_spec_id, equation_id).map(Arc::new),
    )
}

/// Infers and validates the sort of every user equation, populating the
/// `equation_typing` cache (read back during lowering); the first equation
/// that fails inference is returned as the error. The system-defined
/// equations are checked the same way, by [check_system_equations].
pub(crate) fn check_equations(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    system: &UntypedDataSpecification,
) -> Result<(), InferenceError> {
    for eqn_spec in &spec.equation_declarations {
        let eqn_spec_id = eqn_spec.id.expect("assign_declaration_ids ran before check_equations");
        for equation in &eqn_spec.equations {
            let equation_id = equation.id.expect("assign_declaration_ids ran before check_equations");
            // Validate the equation and populate `ctx.equation_typing`; the
            // typing is read back from that cache during lowering.
            query_equation_typing(ctx, spec, system, (eqn_spec_id, equation_id))?;
        }
    }
    Ok(())
}

/// The proven, rigid typing of one Appendix-B template's own equations.
#[derive(Clone)]
pub(crate) struct TemplateCheck {
    pub(crate) type_vars: Vec<TypeVarId>,
    /// Nested the same way as the template's own `equation_declarations`.
    pub(crate) typings: Vec<Vec<Arc<EquationTyping>>>,
}

/// Checks every equation of one Appendix-B container/function-update
/// template, once, with its own `type_var`-declared sort(s) held rigid.
///
/// The returned typing of each equation specializes into every concrete
/// instantiation by substitution.
pub(crate) fn check_template_equations(
    ctx: &mut TypeCheckContext,
    template: &UntypedDataSpecification,
) -> Result<TemplateCheck, InferenceError> {
    let type_vars = template
        .type_var_declarations
        .iter()
        .filter_map(|decl| decl.id)
        .collect();

    let mut typings = Vec::with_capacity(template.equation_declarations.len());
    for eqn_spec in &template.equation_declarations {
        let eqn_spec_id = eqn_spec
            .id
            .expect("assign_declaration_ids ran on the template before check_template_equations");
        let mut block = Vec::with_capacity(eqn_spec.equations.len());
        for equation in &eqn_spec.equations {
            let equation_id = equation
                .id
                .expect("assign_declaration_ids ran on the template before check_template_equations");
            block.push(Arc::new(infer_equation(
                ctx,
                template,
                template,
                EquationRole::Template,
                eqn_spec_id,
                equation_id,
            )?));
        }
        typings.push(block);
    }
    Ok(TemplateCheck { type_vars, typings })
}

/// Specializes a container/function-update template's own proven
/// [EquationTyping] into the typing of one concrete instantiation.
pub(crate) fn specialize_template_typing(
    ctx: &mut TypeCheckContext,
    typing: &EquationTyping,
    vars: &[TypeVarId],
    substitution: &[ResolvedSortId],
) -> EquationTyping {
    debug_assert_eq!(
        vars.len(),
        substitution.len(),
        "one concrete sort per template type variable"
    );
    let substitute = |ctx: &mut TypeCheckContext, sort: ResolvedSortId| {
        vars.iter()
            .zip(substitution)
            .fold(sort, |sort, (&var, &with)| ctx.sorts.substitute_var(sort, var, with))
    };

    let sorts = typing.sorts.iter().map(|&sort| substitute(ctx, sort)).collect();
    let names = typing
        .names
        .iter()
        .map(|(&id, target)| {
            let target = match *target {
                NameTarget::Op { sort } => NameTarget::Op {
                    sort: substitute(ctx, sort),
                },
                other => other,
            };
            (id, target)
        })
        .collect();

    EquationTyping {
        sorts,
        spans: Vec::new(),
        names,
        identifier_names: HashMap::new(),
        declarations: HashMap::new(),
        node_ids: HashMap::new(),
    }
}

/// The system-equation counterpart of [query_equation_typing], memoized on
/// [TypeCheckContext::system_equation_typing].
pub(crate) fn query_system_equation_typing(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    system: &UntypedDataSpecification,
    key: (EqnSpecId, EquationId),
) -> Result<Arc<EquationTyping>, InferenceError> {
    let (eqn_spec_id, equation_id) = key;

    debug_assert!(
        system
            .equation_declarations
            .get(*eqn_spec_id)
            .is_some_and(|eqn_spec| *equation_id < eqn_spec.equations.len()),
        "equation typing key {key:?} must index an equation of the system specification"
    );

    ctx.get_or_compute(
        |ctx| &mut ctx.system_equation_typing,
        key,
        |ctx| infer_equation(ctx, spec, system, EquationRole::System, eqn_spec_id, equation_id).map(Arc::new),
    )
}

/// Infers and validates the sort of every system-defined equation, the same
/// way [check_equations] does for user equations, populating
/// `ctx.system_equation_typing`. Requires `resolve_system_signature_full` to
/// have run, so every binder/equation-variable sort resolves infallibly.
pub(crate) fn check_system_equations(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    system: &UntypedDataSpecification,
    instantiations: &[TemplateInstantiation],
) -> Result<(), InferenceError> {
    // Which template (and local, within-template block index) generated each
    // `system.equation_declarations` block index, if any.
    let mut covered: HashMap<usize, (usize, usize)> = HashMap::new();
    for (instantiation_index, instantiation) in instantiations.iter().enumerate() {
        for (local_index, block_index) in instantiation.equation_range.clone().enumerate() {
            covered.insert(block_index, (instantiation_index, local_index));
        }
    }

    for (block_index, eqn_spec) in system.equation_declarations.iter().enumerate() {
        let eqn_spec_id = eqn_spec
            .id
            .expect("assign_declaration_ids ran on system before check_system_equations");
        match covered.get(&block_index) {
            Some(&(instantiation_index, local_index)) => specialize_instantiation_equations(
                ctx,
                spec,
                eqn_spec,
                eqn_spec_id,
                &instantiations[instantiation_index],
                local_index,
            ),
            None => {
                for equation in &eqn_spec.equations {
                    let equation_id = equation
                        .id
                        .expect("assign_declaration_ids ran on system before check_system_equations");
                    query_system_equation_typing(ctx, spec, system, (eqn_spec_id, equation_id))?;
                }
            }
        }
    }
    Ok(())
}

/// Specializes one generated `EqnSpecId` block's equations.].
fn specialize_instantiation_equations(
    ctx: &mut TypeCheckContext,
    user_spec: &UntypedDataSpecification,
    eqn_spec: &EqnSpec,
    eqn_spec_id: EqnSpecId,
    instantiation: &TemplateInstantiation,
    local_index: usize,
) {
    let check = ctx.template_typings.get(&instantiation.template).unwrap_or_else(|| {
        panic!(
            "template '{}' was not checked before specialization",
            instantiation.template
        )
    });
    let type_vars = check.type_vars.clone();
    let block_typings = check.typings[local_index].clone();

    // Drawn from the user's own already-resolved sort tree, so `resolve_sort`
    // (the one shared resolver, see `docs/typecheck.md`'s `DefId`-offset
    // milestone) resolves every entry infallibly.
    let substitution: Vec<ResolvedSortId> = instantiation
        .substitution
        .iter()
        .map(|sort| resolve_sort(ctx, user_spec, sort))
        .collect();

    for (equation, template_typing) in eqn_spec.equations.iter().zip(&block_typings) {
        let equation_id = equation
            .id
            .expect("assign_declaration_ids ran on system before check_system_equations");
        let specialized = specialize_template_typing(ctx, template_typing, &type_vars, &substitution);
        ctx.system_equation_typing
            .insert((eqn_spec_id, equation_id), Ok(Arc::new(specialized)));
    }
}

/// Resolves the declared sort of one equation-block variable, identified by its own `var_id`. The
/// `System`/`Template` roles are unmemoized: nothing reads a system- or template-equation
/// variable's sort back out later, unlike `DataSpecification::sort_of_equation_var` on the user
/// side — and their own `var_id` numbering is not guaranteed unique against `spec`'s, so sharing
/// `ctx.sort_of_equation_var`'s cache with the `User` role would risk a collision.
fn resolve_equation_variable_sort(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    role: EquationRole,
    var_id: VarId,
    sort: &SortExpression,
) -> ResolvedSortId {
    match role {
        EquationRole::User => query_sort_of_equation_var(ctx, spec, var_id, sort),
        EquationRole::System | EquationRole::Template => resolve_sort(ctx, spec, sort),
    }
}

/// Infers the sorts of a single equation: generates constraints over the
/// condition, left-hand side and right-hand side, solves them by ranked
/// backtracking, and extracts the sorts of the best solution.
///
/// The two sides need not have equal sorts, only a common supersort (either
/// side may be upcast, e.g. `eqn f = 1;` with `f: Nat`), so each side gets a
/// `Sub` constraint against a shared fresh variable.
fn infer_equation(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    system: &UntypedDataSpecification,
    role: EquationRole,
    eqn_spec_id: EqnSpecId,
    equation_id: EquationId,
) -> Result<EquationTyping, InferenceError> {
    // `spec`/`system` are always the true user/system pair; `resolve_sort`
    // resolves a `Resolved` sort's `SortId` against `spec.sort_declarations`
    // regardless of which spec holds the equation (see `docs/typecheck.md`'s
    // `DefId`-offset milestone).
    let eqn_spec = match role {
        EquationRole::User | EquationRole::Template => &spec.equation_declarations[eqn_spec_id],
        EquationRole::System => &system.equation_declarations[eqn_spec_id],
    };
    let equation = &eqn_spec.equations[equation_id];

    // `infer` takes a pre-resolved `(declaration, sort)` scope; resolve each equation variable's
    // sort up front here, keyed by its own `VarId` (assigned by `resolve_data_specification_variables`,
    // for both roles — see `crate::data_specification`).
    let declared_scope: Vec<(VarId, ResolvedSortId)> = eqn_spec
        .variables
        .iter()
        .map(|var| {
            let var_id = var
                .var_id
                .expect("resolve_data_specification_variables ran before check_equations");
            (
                var_id,
                resolve_equation_variable_sort(ctx, spec, role, var_id, &var.sort),
            )
        })
        .collect();

    infer(
        ctx,
        spec,
        role,
        eqn_spec_id,
        &declared_scope,
        Roots::Equation {
            condition: equation.condition.as_ref(),
            lhs: &equation.lhs,
            rhs: &equation.rhs,
        },
        &|| format!("{} = {}", equation.lhs, equation.rhs),
        &equation.span,
    )
}

/// Infers the sorts of one standalone data expression — a term to be
/// rewritten, not part of any equation — against the *user* signature of
/// `spec` (plus the system signature and the full polymorphic scheme table,
/// exactly as a user equation resolves names).
///
/// The expression is closed: it declares no equation variables, so every name
/// in it must resolve to a declared constructor or mapping, and a free
/// identifier is an [`InferenceError::UndeclaredName`]. Bound variables
/// introduced by a `lambda`/`forall`/`exists`/comprehension/`whr` inside the
/// expression are unaffected — those carry their own declared sorts.
///
/// Unlike an equation there is no second side to widen against, so the
/// expression's sort follows from its own structure alone: `1 + 1` infers at
/// `Pos`, the minimal sort the ranked search admits.
pub(crate) fn infer_expression(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    expr: &DataExpr,
) -> Result<EquationTyping, InferenceError> {
    infer_expression_in_scope(ctx, spec, expr, &[], None)
}

/// As [`infer_expression`], but against an externally-supplied `declared_scope` (rather than
/// none) and, when `expected` is given, additionally constrained to be a subsort of it (rather
/// than typed purely from the expression's own structure) — see [`Roots::ExpressionAgainst`].
///
/// Used by [`crate::process`]/[`crate::pbes`] to check an action argument, a process-instantiation
/// argument, a `sum`/`dist` condition or time bound, a `PropVarInst` argument, or similar, each
/// against its own already-known expected sort. `declared_scope` covers every global variable,
/// process/PBES parameter, and `sum`/`dist`/quantifier binder in scope, keyed by each one's own
/// declaration [VarId] (see [`infer`]'s doc comment).
pub(crate) fn infer_expression_in_scope(
    ctx: &mut TypeCheckContext,
    spec: &UntypedDataSpecification,
    expr: &DataExpr,
    declared_scope: &[(VarId, ResolvedSortId)],
    expected: Option<ResolvedSortId>,
) -> Result<EquationTyping, InferenceError> {
    let roots = match expected {
        Some(expected) => Roots::ExpressionAgainst { expr, expected },
        None => Roots::Expression(expr),
    };
    infer(
        ctx,
        spec,
        EquationRole::User,
        // Unused: the `User` role reads no per-group state, and a scope here is never an
        // equation's `var` block.
        EqnSpecId::new(0),
        declared_scope,
        roots,
        &|| expr.to_string(),
        &expr.span,
    )
}

/// The expressions one inference run covers.
enum Roots<'a> {
    /// The three sides of an equation, joined through a common supersort.
    Equation {
        condition: Option<&'a DataExpr>,
        lhs: &'a DataExpr,
        rhs: &'a DataExpr,
    },
    /// A single standalone expression, typed on its own (see [infer_expression]).
    Expression(&'a DataExpr),
    /// A single expression checked against an already-known expected sort — an action argument
    /// or a process-instantiation argument's declared sort, a `sum`/`dist` condition or time
    /// bound — rather than typed purely from its own structure (see
    /// [infer_expression_in_scope]/[crate::process]).
    ExpressionAgainst {
        expr: &'a DataExpr,
        expected: ResolvedSortId,
    },
}

/// Generates the constraints of `roots`, solves them by ranked backtracking,
/// and extracts the sorts of the best solution. Shared by [infer_equation],
/// [infer_expression], and [infer_expression_in_scope]; `text` renders the whole input for
/// diagnostics and `span` locates it in the source.
///
/// `declared_scope` is a pre-resolved `(declaration VarId, sort)` list, resolved by the caller
/// before this is called — an equation's own `var`-block variables (see [infer_equation]) and a
/// process/PBES/PRES scope ([infer_expression_in_scope]/[crate::process]) alike, each looked up by
/// a `Resolved` node's own declaration [VarId]: [`crate::resolve_data_specification_variables`]/
/// [`crate::resolve_process_variables`]/[`crate::resolve_pbes_variables`] tie every such occurrence
/// to its declaration during variable resolution, before inference ever runs.
#[allow(clippy::too_many_arguments)]
fn infer<'a>(
    ctx: &mut TypeCheckContext,
    spec: &'a UntypedDataSpecification,
    role: EquationRole,
    eqn_spec_id: EqnSpecId,
    declared_scope: &[(VarId, ResolvedSortId)],
    roots: Roots<'a>,
    equation_text: &dyn Fn() -> String,
    equation_span: &Span,
) -> Result<EquationTyping, InferenceError> {
    debug!("inference: typing '{}'", equation_text());

    let mut unifier = Unifier::new();

    // A `Resolved` node's own declaration `VarId` looks itself up here directly.
    let mut declared_sorts = HashMap::new();
    for &(declaration, sort) in declared_scope {
        let node = unifier.resolved_node(sort);
        declared_sorts.insert(declaration, node);
    }

    // The signatures are cloned out of the context (cheaply, behind `Arc`)
    // because the generator needs the context mutably: resolving a
    // comprehension's binder sort interns sorts and fills the sort-of-def
    // cache mid-walk.
    //
    // `builtin_schemes` is the *only* remaining source of polymorphic
    // overloads for the System role: `ctx.basics_signature` and a struct's own override never
    // carry schemes (only `ctx.signature` does, merged in by `resolve_system_signature`/
    // `build_signature`), and the System role deliberately does *not* fall back to `ctx.signature`
    // itself — doing so would let a struct's own equation see every *other* user declaration too
    // (including an unrelated struct's same-named constructor/projection), not just the basic-sort
    // operators it actually needs. See `docs/typecheck.md`'s trusted-signature milestone.
    let (signature, builtin_schemes): (Arc<Signature>, Arc<HashMap<String, Vec<PolySortScheme>>>) = match role {
        EquationRole::User | EquationRole::Template => (
            Arc::clone(ctx.signature.as_ref().expect("build_signature ran before inference")),
            Arc::new(HashMap::new()),
        ),
        EquationRole::System => (
            ctx.struct_signature_overrides
                .get(&eqn_spec_id)
                .cloned()
                .unwrap_or_else(|| {
                    Arc::clone(
                        ctx.basics_signature
                            .as_ref()
                            .expect("resolve_system_signature ran before inference"),
                    )
                }),
            build_builtin_scheme_signature(ctx),
        ),
    };
    let system_signature = Arc::clone(
        ctx.basics_signature
            .as_ref()
            .expect("resolve_system_signature ran before inference"),
    );

    let mut generator = ConstraintGenerator {
        ctx: &mut *ctx,
        spec,
        signature,
        system_signature,
        builtin_schemes,
        declared_sorts,
        unifier: &mut unifier,
        expr_sorts: Vec::new(),
        expr_texts: Vec::new(),
        log_texts: log::log_enabled!(log::Level::Debug),
        expr_spans: Vec::new(),
        expr_names: HashMap::new(),
        expr_declarations: HashMap::new(),
        expr_ids: HashMap::new(),
        collect_typing_info: matches!(role, EquationRole::User),
        names: HashMap::new(),
        constraints: Vec::new(),
    };

    // Captured before `roots` is consumed below, so a `NoTyping` failure can blame the sort
    // inference was actually asked to match, when one was given.
    let expected_sort = match &roots {
        Roots::ExpressionAgainst { expected, .. } => Some(*expected),
        Roots::Equation { .. } | Roots::Expression(_) => None,
    };

    let generated = match roots {
        Roots::Equation { condition, lhs, rhs } => generator.generate(condition, lhs, rhs),
        Roots::Expression(expr) => generator.generate_expression(expr),
        Roots::ExpressionAgainst { expr, expected } => generator.generate_against(expr, expected),
    };
    match generated {
        Ok(()) => {}
        Err(GenFailure::InvalidBinderSort(sort, span)) => {
            debug!(
                "inference: rejected '{}', its binder sort '{sort}' is not a valid variable sort",
                equation_text()
            );
            return Err(InferenceError::InvalidBinderSort {
                sort,
                expression: equation_text(),
                span,
            });
        }
        Err(GenFailure::Error(error)) => {
            debug!(
                "inference: constraint generation failed for '{}': {error}",
                equation_text()
            );
            return Err(error);
        }
    }

    // Drop the generator's borrow of the context; the solver needs the
    // interner mutably to intern widened and extracted sorts.
    let ConstraintGenerator {
        expr_sorts,
        expr_texts,
        expr_spans,
        expr_names,
        expr_declarations,
        expr_ids,
        names,
        constraints,
        ..
    } = generator;

    // Merge the `Sub`s that widen into a shared free variable into one `Join`,
    // so their common supersort is computed in one step instead of order-
    // sensitively.
    let constraints = merge_shared_subs(constraints, &mut unifier);
    trace!(
        "inference: generated {} constraint(s) over {} expression node(s)",
        constraints.len(),
        expr_sorts.len()
    );

    let mut solver = Solver {
        sorts: &mut ctx.sorts,
        unifier: &mut unifier,
        constraints: &constraints,
        expr_sorts: &expr_sorts,
        base_names: &names,
        choices: Vec::new(),
        measure: Vec::new(),
        best: None,
    };
    solver.solve(0);

    // A push/pop mismatch on a dead-end branch never reaches `leaf()`'s
    // balance check, yet would corrupt the measure prefix of every later
    // branch — a silently wrong "best" typing rather than a crash.
    debug_assert!(
        solver.measure.is_empty() && solver.choices.is_empty(),
        "the solver unwinds its measure and choice stacks"
    );

    match solver.best {
        None => {
            debug!("inference: no valid sort assignment for '{}'", equation_text());
            Err(InferenceError::NoTyping {
                expression: equation_text(),
                sort: expected_sort.map(|sort| DisplaySortContext::new(ctx, spec, sort).to_string()),
                span: equation_span.clone(),
            })
        }
        Some(best) if best.duplicate => {
            debug!(
                "inference: two solutions tie at measure {:?} for '{}'",
                best.measure,
                equation_text()
            );
            Err(InferenceError::AmbiguousExpression {
                expression: equation_text(),
                span: equation_span.clone(),
            })
        }
        Some(best) => match best.typing {
            None => {
                debug!(
                    "inference: the best solution leaves a sort free in '{}'",
                    equation_text()
                );
                Err(InferenceError::UnderdeterminedSort {
                    expression: equation_text(),
                    span: equation_span.clone(),
                })
            }
            Some((sorts, names)) => {
                // The contract of the Phase-4 side tables: one sort per
                // expression node, and name targets only for existing nodes.
                debug_assert_eq!(sorts.len(), expr_sorts.len(), "one inferred sort per expression node");
                debug_assert!(
                    names.keys().all(|id| **id < sorts.len()),
                    "every name target keys an expression node"
                );
                debug_assert!(
                    expr_texts.is_empty() || expr_texts.len() == sorts.len(),
                    "expr_texts is parallel to the expression nodes when filled"
                );
                debug_assert!(
                    expr_spans.is_empty() || expr_spans.len() == sorts.len(),
                    "expr_spans is parallel to the expression nodes when filled"
                );
                debug_assert!(
                    expr_names.keys().all(|id| **id < sorts.len()),
                    "every recorded identifier name keys an expression node"
                );

                debug!("inference: solved '{}' at measure {:?}", equation_text(), best.measure);
                if log::log_enabled!(log::Level::Debug) {
                    for &(declaration, sort) in declared_scope {
                        trace!(
                            "inference:   variable {declaration:?}: {}",
                            DisplaySortContext::new(ctx, spec, sort)
                        );
                    }
                    for (&sort, text) in sorts.iter().zip(&expr_texts) {
                        trace!("inference:   '{text}': {}", DisplaySortContext::new(ctx, spec, sort));
                    }
                }
                Ok(EquationTyping {
                    sorts,
                    spans: expr_spans,
                    names,
                    identifier_names: expr_names,
                    declarations: expr_declarations,
                    node_ids: expr_ids,
                })
            }
        },
    }
}

/// Merges every group of `Sub` constraints that widen into the same free
/// variable into a single [Join] at the position of the group's last member.
/// A free variable shared by two or more `Sub` targets is an eagerly-unified
/// parameter — a scheme operand (`==`/`!=`/`<`/…/`if`), a set/bag element, or
/// the equation's LHS/RHS join — whose sequential greedy widening is
/// order-sensitive and can force a fruitless re-exploration. The join computes
/// their least common supersort in one step instead. A
/// disjunction overload's parameters are *distinct* fresh variables (the
/// overload is not committed until solving), so they are never grouped and the
/// argument-before-callee pruning of [Constraint] is preserved.
fn merge_shared_subs(constraints: Vec<Constraint>, unifier: &mut Unifier) -> Vec<Constraint> {
    // Group the `Sub` indices by the union-find root of their free-variable
    // target. A bound (concrete) target has no root and is never grouped.
    let mut groups: HashMap<u32, Vec<usize>> = HashMap::new();
    for (index, constraint) in constraints.iter().enumerate() {
        if let Constraint::Sub(sub) = constraint
            && let Some(root) = unifier.free_root(sub.rhs)
        {
            groups.entry(root).or_default().push(index);
        }
    }

    // Build one `Join` per group of two or more, placed at the last member's
    // position (by then every source's own sort is determined); the earlier
    // members are dropped.
    let mut joins: HashMap<usize, Join> = HashMap::new();
    let mut dropped = vec![false; constraints.len()];
    for indices in groups.into_values() {
        if indices.len() < 2 {
            continue;
        }
        let sources = indices
            .iter()
            .map(|&index| match &constraints[index] {
                Constraint::Sub(sub) => sub.lhs,
                _ => unreachable!("only Sub indices are grouped"),
            })
            .collect();
        let last = *indices.last().expect("a merged group has at least two members");
        // Every member's target denotes the same shared class; take one.
        let target = match &constraints[last] {
            Constraint::Sub(sub) => sub.rhs,
            _ => unreachable!("only Sub indices are grouped"),
        };
        for &index in &indices {
            dropped[index] = true;
        }
        joins.insert(last, Join { sources, target });
    }

    if joins.is_empty() {
        return constraints;
    }

    // Rebuild in the original order: a grouped `Sub` becomes its group's `Join`
    // at the last member and vanishes at the earlier members; everything else
    // is untouched.
    let mut result = Vec::with_capacity(constraints.len());
    for (index, constraint) in constraints.into_iter().enumerate() {
        if let Some(join) = joins.remove(&index) {
            result.push(Constraint::Join(join));
        } else if !dropped[index] {
            result.push(constraint);
        }
    }
    result
}

/// The kind of a number literal: `0` is natural, every other literal positive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LitKind {
    Positive,
    Natural,
}

/// Requires the sort of `lhs` to be a subsort of `rhs`, modelling the implicit
/// upcasts that mCRL2 inserts (a `Nat` argument may be passed where `Int` is
/// expected).
struct SubConstraint {
    lhs: InferSortId,
    rhs: InferSortId,
}

/// Requires `sort` to be a number sort admitting the literal kind.
struct LitConstraint {
    sort: InferSortId,
    kind: LitKind,
}

/// The overload set of a name with several candidates; the solver commits to
/// exactly one disjunct per solution.
struct Disjunction {
    /// The `Id` node whose chosen [NameTarget] is recorded per solution.
    expr: ExprId,
    /// The sort node of that `Id` node.
    sort: InferSortId,
    disjuncts: Vec<(NameTarget, InferSortId)>,
}

/// The two readings of a set/bag comprehension `{ x: S | e }`: a `Bool` body
/// denotes a `Set(S)`, a `Nat` or `Pos` body the multiplicities of a `Bag(S)`.
/// Which reading applies follows from the solved body sort; Phase-4 lowering
/// derives the binder kind from the node's sort and inserts the `Pos` → `Nat`
/// coercion on a positive body.
struct Comprehension {
    /// The sort node of the predicate (or count) body.
    body: InferSortId,
    /// The sort node of the comprehension expression itself.
    node: InferSortId,
    /// The resolved sort of the bound variable.
    element: ResolvedSortId,
}

/// A least-upper-bound constraint: the `target` sort is the lattice join of
/// the `sources` (their least common supersort). It replaces a group of
/// individual `Sub` constraints that all widen into the *same* free variable —
/// the eagerly-shared parameter of a scheme (`==`/`!=`/`<`/…/`if`), a set or
/// bag element, or the equation's LHS/RHS join. Sequential `Sub`s are
/// order-sensitive: whichever source is solved first binds the shared variable
/// by equality, so a finite-container source (a set literal, `FSet`) fixes the
/// result to `FSet` before another source (a comprehension, `Set`) is typed,
/// which then cannot satisfy its own `Sub` and forces the whole comprehension
/// body to be re-explored fruitlessly. Computing the join directly picks the
/// common supersort in one step, with no premature commitment, while
/// contributing the same per-source widening measure as the `Sub`s did — so
/// the ranking is unchanged. Built by [merge_shared_subs] after constraint
/// generation.
struct Join {
    /// The branch sort nodes joined into `target`, in generation order.
    sources: Vec<InferSortId>,
    /// The shared variable the sources widen into (e.g. an `if`'s result).
    target: InferSortId,
}

/// One constraint of an equation, solved in generation order. Interleaving the
/// kinds (rather than deciding all disjunctions first) is what keeps the
/// search tractable: the arguments of an application are generated before its
/// callee, so by the time a callee's overload disjunction is tried, the
/// argument sorts are already bound and most disjuncts fail to unify at once.
enum Constraint {
    Sub(SubConstraint),
    Lit(LitConstraint),
    Disjunction(Disjunction),
    Comprehension(Comprehension),
    Join(Join),
}

/// Why constraint generation stopped early.
enum GenFailure {
    /// A binder in the equation declares a sort that is not a valid variable
    /// sort (a bare product; see [is_supported_binder_sort]). The equation is
    /// rejected rather than left untyped. Carries the offending sort's text
    /// and the span of the binder that declares it.
    InvalidBinderSort(String, Span),
    Error(InferenceError),
}

/// Walks the lowered expressions of one equation, assigning [ExprId]s and
/// emitting the constraints. Structural facts that must hold in every solution
/// (a callee has a function sort, the condition is boolean) are unified
/// eagerly, so their failure is a direct error rather than a solver miss.
struct ConstraintGenerator<'a> {
    /// Mutable so a comprehension's binder sort can be resolved (interned)
    /// mid-walk; the signatures below are `Arc` clones out of this same context.
    ctx: &'a mut TypeCheckContext,
    /// Always the true user spec, regardless of the equation role `infer` built this from.
    spec: &'a UntypedDataSpecification,
    signature: Arc<Signature>,
    /// Always the basic-sort system signature, regardless of `role`.
    system_signature: Arc<Signature>,
    /// The narrow comparison/`if`-only scheme table consulted alongside
    /// `signature`'s own `schemes`; see [`infer`]'s construction of this
    /// field for why it differs by role. Empty for the User role, since
    /// `ctx.signature.schemes` already covers everything polymorphic.
    builtin_schemes: Arc<HashMap<String, Vec<PolySortScheme>>>,
    /// A `Resolved` node's declaration [VarId], mapped to its sort; see [`infer`]'s doc comment.
    declared_sorts: HashMap<VarId, InferSortId>,
    unifier: &'a mut Unifier,
    /// The sort node of every expression, indexed by [ExprId].
    expr_sorts: Vec<InferSortId>,
    /// The display text of every expression, parallel to `expr_sorts`; only
    /// filled when [Self::log_texts], to report the solved typing.
    expr_texts: Vec<String>,
    /// Whether debug logging was enabled when generation started; sampled once
    /// so `expr_texts` stays parallel to `expr_sorts` even under a log filter
    /// that changes mid-equation.
    log_texts: bool,
    /// The source span of every expression, parallel to `expr_sorts`; only
    /// filled when [Self::collect_typing_info]. Becomes [EquationTyping::spans].
    expr_spans: Vec<Span>,
    /// The identifier text of every `Id` node, keyed by its [ExprId]; only
    /// filled when [Self::collect_typing_info]. Becomes
    /// [EquationTyping::identifier_names].
    expr_names: HashMap<ExprId, String>,
    /// The declaration [VarId] of every `Resolved` node — a variable reference that names its own
    /// binder — keyed by its [ExprId]; only filled when [Self::collect_typing_info]. Becomes
    /// [EquationTyping::declarations].
    expr_declarations: HashMap<ExprId, VarId>,
    /// Every visited node's own [ExprId], keyed by that node's address; only filled when
    /// [Self::collect_typing_info]. Becomes [EquationTyping::node_ids].
    expr_ids: HashMap<usize, ExprId>,
    /// Whether [Self::expr_spans]/[Self::expr_names] should be filled — i.e.
    /// whether `role` is [EquationRole::User]. Sampled once at construction.
    collect_typing_info: bool,
    /// The targets of names resolved during generation (variables and
    /// single-candidate names); disjunction choices are added by the solver.
    names: HashMap<ExprId, NameTarget>,
    constraints: Vec<Constraint>,
}

impl<'a> ConstraintGenerator<'a> {
    fn generate(
        &mut self,
        condition: Option<&'a DataExpr>,
        lhs: &'a DataExpr,
        rhs: &'a DataExpr,
    ) -> Result<(), GenFailure> {
        // Checked once at the roots: the property is subtree-closed, and
        // re-checking per node in `visit` would be quadratic on the deep
        // expressions where solving is already expensive.
        debug_assert!(
            condition.is_none_or(is_lowered) && is_lowered(lhs) && is_lowered(rhs),
            "inference requires lowered expressions"
        );

        if let Some(condition) = condition {
            let sort = self.visit(condition)?;
            let bool_node = self.unifier.resolved_node(self.ctx.sorts.bool_sort());
            if !self.unifier.unify(&self.ctx.sorts, sort, bool_node) {
                return Err(GenFailure::Error(InferenceError::ConditionNotBool {
                    condition: condition.to_string(),
                    span: condition.span.clone(),
                }));
            }
        }

        let lhs_sort = self.visit(lhs)?;
        let rhs_sort = self.visit(rhs)?;
        let joined = self.unifier.fresh_var();
        self.constraints.push(Constraint::Sub(SubConstraint {
            lhs: lhs_sort,
            rhs: joined,
        }));
        self.constraints.push(Constraint::Sub(SubConstraint {
            lhs: rhs_sort,
            rhs: joined,
        }));
        Ok(())
    }

    /// Emits the constraints of a single standalone expression (see
    /// [infer_expression]). Unlike [Self::generate] there is no second side to
    /// widen against, so no `Sub` into a shared variable is added and the
    /// expression's sort is whatever its own structure admits.
    fn generate_expression(&mut self, expr: &'a DataExpr) -> Result<(), GenFailure> {
        debug_assert!(is_lowered(expr), "inference requires lowered expressions");

        self.visit(expr)?;
        Ok(())
    }

    /// As [`Self::generate_expression`], but additionally constrains `expr`'s sort to be a
    /// subsort of `expected`, which must already be concrete (not a fresh variable).
    fn generate_against(&mut self, expr: &'a DataExpr, expected: ResolvedSortId) -> Result<(), GenFailure> {
        debug_assert!(is_lowered(expr), "inference requires lowered expressions");

        let sort = self.visit(expr)?;
        let target = self.unifier.resolved_node(expected);
        self.constraints
            .push(Constraint::Sub(SubConstraint { lhs: sort, rhs: target }));
        Ok(())
    }

    /// Emits the constraints for `expr` and returns its sort node: a fresh
    /// variable constrained by the expression form.
    fn visit(&mut self, expr: &'a DataExpr) -> Result<InferSortId, GenFailure> {
        let node = self.unifier.fresh_var();
        let id = ExprId::new(self.expr_sorts.len());
        self.expr_sorts.push(node);
        if self.log_texts {
            self.expr_texts.push(expr.to_string());
        }
        if self.collect_typing_info {
            self.expr_spans.push(expr.span.clone());
            self.expr_ids.insert(expr as *const DataExpr as usize, id);
        }

        match &expr.node {
            DataExprKind::Id(name) => self.gen_name(id, node, name, None, &expr.span)?,
            DataExprKind::Resolved(name, declaration) => {
                self.gen_name(id, node, name, Some(*declaration), &expr.span)?
            }
            DataExprKind::Number(value) => {
                let kind = if value == "0" {
                    LitKind::Natural
                } else {
                    LitKind::Positive
                };
                self.constraints
                    .push(Constraint::Lit(LitConstraint { sort: node, kind }));
            }
            DataExprKind::Bool(_) => {
                let bool_node = self.unifier.resolved_node(self.ctx.sorts.bool_sort());
                self.bind_fresh(node, bool_node);
            }
            DataExprKind::EmptyList => {
                let element = self.unifier.fresh_var();
                let list = self.unifier.generic(ComplexSort::List, element);
                self.bind_fresh(node, list);
            }
            // The enumerated and empty set/bag literals take the *finite*
            // container sort; where a `Set`/`Bag` is expected, the sub-sort
            // constraints widen `FSet(S) <= Set(S)` (`FBag(S) <= Bag(S)`) at
            // the point of use, matching mCRL2's upcast of an enumeration.
            DataExprKind::EmptySet => {
                let element = self.unifier.fresh_var();
                let set = self.unifier.generic(ComplexSort::FSet, element);
                self.bind_fresh(node, set);
            }
            DataExprKind::EmptyBag => {
                let element = self.unifier.fresh_var();
                let bag = self.unifier.generic(ComplexSort::FBag, element);
                self.bind_fresh(node, bag);
            }
            DataExprKind::Set(members) => {
                // The members share one element node into which each may be
                // upcast, so the solved element sort is the least common
                // supersort of the member sorts.
                let element = self.unifier.fresh_var();
                for member in members {
                    let member_sort = self.visit(member)?;
                    self.constraints.push(Constraint::Sub(SubConstraint {
                        lhs: member_sort,
                        rhs: element,
                    }));
                }
                let set = self.unifier.generic(ComplexSort::FSet, element);
                self.bind_fresh(node, set);
            }
            DataExprKind::Bag(members) => {
                let element = self.unifier.fresh_var();
                let nat = self.unifier.resolved_node(self.ctx.sorts.nat_sort());
                for member in members {
                    let member_sort = self.visit(&member.expr)?;
                    self.constraints.push(Constraint::Sub(SubConstraint {
                        lhs: member_sort,
                        rhs: element,
                    }));
                    // A multiplicity is a natural number; a `Pos` count is
                    // upcast, anything else is an error.
                    let count_sort = self.visit(&member.multiplicity)?;
                    self.constraints.push(Constraint::Sub(SubConstraint {
                        lhs: count_sort,
                        rhs: nat,
                    }));
                }
                let bag = self.unifier.generic(ComplexSort::FBag, element);
                self.bind_fresh(node, bag);
            }
            DataExprKind::SetBagComp { variable, predicate } => {
                // The bound variable is in scope for the predicate only; it has no [ExprId] of
                // its own, like every other binder here.
                let comprehension = self.with_binder_scope(std::slice::from_ref(variable), |this, sorts| {
                    let body = this.visit(predicate)?;
                    Ok(Comprehension {
                        body,
                        node,
                        element: sorts[0],
                    })
                })?;
                self.constraints.push(Constraint::Comprehension(comprehension));
            }
            DataExprKind::Application { function, arguments } => {
                // The arguments are visited (and hence constrained) before the
                // applied function, so the function's overload disjunction is
                // solved against already-bound argument sorts.
                let mut parameters = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    let argument_sort = self.visit(argument)?;
                    // The parameter is a fresh variable rather than the
                    // argument sort itself, so the argument may be upcast into
                    // the parameter the overload expects.
                    let parameter = self.unifier.fresh_var();
                    self.constraints.push(Constraint::Sub(SubConstraint {
                        lhs: argument_sort,
                        rhs: parameter,
                    }));
                    parameters.push(parameter);
                }
                let function_sort = self.visit(function)?;
                let expected = self.unifier.function(parameters, node);
                if !self.unifier.unify(&self.ctx.sorts, function_sort, expected) {
                    return Err(GenFailure::Error(InferenceError::NotAFunction {
                        expr: function.to_string(),
                        span: function.span.clone(),
                    }));
                }
            }
            DataExprKind::Lambda { variables, body } => {
                // The result is a function from the bound variables' declared
                // sorts to the body's sort.
                let function_sort = self.with_binder_scope(variables, |this, sorts| {
                    let body_sort = this.visit(body)?;
                    let parameters = sorts.iter().map(|&sort| this.unifier.resolved_node(sort)).collect();
                    Ok(this.unifier.function(parameters, body_sort))
                })?;
                self.bind_fresh(node, function_sort);
            }
            DataExprKind::Quantifier { op: _, variables, body } => {
                // A `forall`/`exists` is `Bool`, and requires its body to be
                // `Bool` too (mCRL2 checks both with `TypeMatchA(Bool, ..)`).
                let bool_node = self.unifier.resolved_node(self.ctx.sorts.bool_sort());
                self.with_binder_scope(variables, |this, _sorts| {
                    let body_sort = this.visit(body)?;
                    if !this.unifier.unify(&this.ctx.sorts, body_sort, bool_node) {
                        return Err(GenFailure::Error(InferenceError::QuantifierNotBool {
                            body: body.to_string(),
                            span: body.span.clone(),
                        }));
                    }
                    Ok(())
                })?;
                self.bind_fresh(node, bool_node);
            }
            DataExprKind::Whr { expr, assignments } => {
                // Each assignment's right-hand side is typed in the outer
                // scope — bindings do not see each other, only the body does
                // (every assignment is typed against the original declared
                // variables, the context being extended once, for the body).
                // So every right-hand side is visited first, and only
                // then are the names shadowed as a batch.
                // The bound variable's sort is the assignment's own inferred
                // sort node, so it has no [ExprId] of its own to resolve a
                // declared sort against, unlike a comprehension/lambda/quantifier binder — its
                // declared sort *is* that inferred node.
                let mut bindings = Vec::with_capacity(assignments.len());
                for assignment in assignments {
                    let value_node = self.visit(&assignment.expr)?;
                    let var_id = assignment.id.expect(
                        "resolve_data_specification_variables/resolve_process_variables/... ran before inference",
                    );
                    bindings.push((var_id, value_node));
                }
                for &(var_id, value_node) in &bindings {
                    self.declared_sorts.insert(var_id, value_node);
                }
                let body_sort = self.visit(expr)?;
                for &(var_id, _) in &bindings {
                    self.declared_sorts.remove(&var_id);
                }
                self.bind_fresh(node, body_sort);
            }
            DataExprKind::List(_)
            | DataExprKind::Unary { .. }
            | DataExprKind::Binary { .. }
            | DataExprKind::FunctionUpdate { .. } => {
                unreachable!("lowering rewrote this expression form")
            }
        }

        Ok(node)
    }

    /// Binds the fresh sort node of the current expression to `sort`;
    /// infallible because a fresh variable unifies with any sort.
    fn bind_fresh(&mut self, node: InferSortId, sort: InferSortId) {
        let unified = self.unifier.unify(&self.ctx.sorts, node, sort);
        debug_assert!(unified, "a fresh variable unifies with any sort");
    }

    /// Resolves the declared sort of each of `variables` (rejecting an invalid binder sort, see
    /// [Self::binder_sort]), registers each by its own [VarId] in `self.declared_sorts` for the
    /// duration of `f`, then removes them again. Unlike a name-keyed scope this never needs to
    /// save/restore a shadowed binding: every binder has its own `VarId`, so nested binders of the
    /// same name can never collide here. Used for every binder that declares its variables' own
    /// sorts — `lambda`, `forall`/`exists`, and a set/bag comprehension's single bound variable —
    /// unlike a `whr` binding, whose sort follows from its right-hand side instead.
    fn with_binder_scope<T>(
        &mut self,
        variables: &'a [IdDecl],
        f: impl FnOnce(&mut Self, &[ResolvedSortId]) -> Result<T, GenFailure>,
    ) -> Result<T, GenFailure> {
        let mut sorts = Vec::with_capacity(variables.len());
        let mut var_ids = Vec::with_capacity(variables.len());
        for variable in variables {
            let sort = self.binder_sort(&variable.sort, &variable.identifier.span)?;
            let node = self.unifier.resolved_node(sort);
            let var_id = variable
                .var_id
                .expect("resolve_data_specification_variables/resolve_process_variables/... ran before inference");
            self.declared_sorts.insert(var_id, node);
            var_ids.push(var_id);
            sorts.push(sort);
        }

        let result = f(self, &sorts);

        for var_id in var_ids {
            self.declared_sorts.remove(&var_id);
        }

        result
    }

    /// Resolves the declared sort of a comprehension's bound variable onto the
    /// interned lattice, rejecting a sort that is not a valid variable sort
    /// (a bare product; see [is_supported_binder_sort]). `span` is the
    /// binder's declaration span, reported on rejection.
    fn binder_sort(&mut self, sort: &SortExpression, span: &Span) -> Result<ResolvedSortId, GenFailure> {
        if !is_supported_binder_sort(sort) {
            return Err(GenFailure::InvalidBinderSort(sort.to_string(), span.clone()));
        }
        Ok(resolve_sort(self.ctx, self.spec, sort))
    }

    /// Resolves the candidates of a name: a `Resolved` node's own declaration (`declaration`, its
    /// binder's own [VarId], looked up in `self.declared_sorts`) shadows everything — every
    /// binder in scope, whether introduced elsewhere in this same expression (a
    /// `lambda`/`forall`/`exists`/comprehension/`whr` binder) or outside it (an equation's own
    /// `var`-block variable, a process/PBES/PRES parameter, a global) is registered there by
    /// variable resolution or [Self::with_binder_scope] before this ever runs — then the user
    /// overloads joined by the system-defined overloads and the polymorphic built-in schemes (the
    /// container and function-update operations, the comparison operators and `if`), each
    /// instantiated fresh per occurrence.
    ///
    /// `declaration` is `Some` exactly when this occurrence is a `Resolved` node, carrying its
    /// binder's own [VarId]; it is also always recorded (when `Some`, regardless of which
    /// candidate `name` resolves to), becoming
    /// `ResolvedName::Variable`'s `declaration` in `typing_info`.
    fn gen_name(
        &mut self,
        id: ExprId,
        node: InferSortId,
        name: &'a str,
        declaration: Option<VarId>,
        span: &Span,
    ) -> Result<(), GenFailure> {
        // Recorded before resolving which candidate `name` refers to; the disjunction case is
        // resolved later by the solver, not here.
        if self.collect_typing_info {
            self.expr_names.insert(id, name.to_string());
            if let Some(declaration) = declaration {
                self.expr_declarations.insert(id, declaration);
            }
        }

        if let Some(declaration) = declaration {
            debug_assert!(
                self.declared_sorts.contains_key(&declaration),
                "{declaration:?} (occurrence {name:?}) was resolved to a `DataExprKind::Resolved` \
                 by the variable-resolution pre-pass but has no entry in `declared_sorts` — \
                 `checking::Scope`/`with_binder_scope` and the pre-pass have gone out of sync"
            );
        }

        if let Some(sort) = declaration.and_then(|declaration| self.declared_sorts.get(&declaration)) {
            self.names.insert(id, NameTarget::Variable);
            self.bind_fresh(node, *sort);
            return Ok(());
        }

        let mut disjuncts: Vec<(NameTarget, InferSortId)> = Vec::new();
        let signature = Arc::clone(&self.signature);
        let system_signature = Arc::clone(&self.system_signature);
        self.push_signature_disjuncts(&signature, name, &mut disjuncts);
        self.push_signature_disjuncts(&system_signature, name, &mut disjuncts);

        // The comparison operators and `if` are not in either signature above
        // for the System role (see `builtin_schemes`'s doc comment); for the
        // User role this is always empty, since `self.signature.schemes`
        // (part of `ctx.signature`, pushed above) already covers everything
        // polymorphic. Each scheme overload is instantiated with fresh
        // variables per occurrence, mirroring mCRL2's polymorphic symbol
        // table; Phase-4 lowering recovers the concrete operation from the
        // name and the inferred sort.
        //
        // Collected into a `Vec` first (rather than iterating `self.builtin_schemes` directly) so
        // the loop below can call `self.instantiate_scheme`, which needs `&mut self`, without
        // holding a borrow into `self.builtin_schemes` across it.
        let builtin_sorts: Vec<ResolvedSortId> = self
            .builtin_schemes
            .get(name)
            .into_iter()
            .flatten()
            .map(|scheme| scheme.sort)
            .collect();
        for sort in builtin_sorts {
            let instance = self.instantiate_scheme(sort, &mut HashMap::new());
            disjuncts.push((NameTarget::Builtin, instance));
        }

        match disjuncts.as_slice() {
            [] => Err(GenFailure::Error(InferenceError::UndeclaredName {
                name: name.to_string(),
                span: span.clone(),
            })),
            [(target, sort)] => {
                self.names.insert(id, *target);
                self.bind_fresh(node, *sort);
                Ok(())
            }
            _ => {
                trace!("inference: name '{name}' has {} candidate(s)", disjuncts.len());
                self.constraints.push(Constraint::Disjunction(Disjunction {
                    expr: id,
                    sort: node,
                    disjuncts,
                }));
                Ok(())
            }
        }
    }

    /// Pushes every overload of `name` found in `signature` — ground
    /// (`constructors`/`mappings`) and polymorphic (`schemes`) alike — onto
    /// `disjuncts`. `signature` is taken by value (an `Arc` clone, cheap) so
    /// this can call [Self::instantiate_scheme] (which needs `&mut self`)
    /// without borrowing `self.signature`/`self.system_signature` for the
    /// duration.
    fn push_signature_disjuncts(
        &mut self,
        signature: &Arc<Signature>,
        name: &str,
        disjuncts: &mut Vec<(NameTarget, InferSortId)>,
    ) {
        for overloads in [signature.constructors.get(name), signature.mappings.get(name)]
            .into_iter()
            .flatten()
        {
            for &overload in overloads {
                let target = NameTarget::Op { sort: overload };
                // The user and system specifications may declare the same
                // symbol; a duplicate disjunct would misreport ambiguity.
                if !disjuncts.iter().any(|(existing, _)| *existing == target) {
                    disjuncts.push((target, self.unifier.resolved_node(overload)));
                }
            }
        }
        for scheme in signature.schemes.get(name).into_iter().flatten() {
            let instance = self.instantiate_scheme(scheme.sort, &mut HashMap::new());
            disjuncts.push((NameTarget::Builtin, instance));
        }
    }

    /// A fresh instance of a scheme's already-*interned* sort: every bound
    /// [`ResolvedSort::Var`] it mentions becomes one fresh unification
    /// variable, shared between its occurrences within this one
    /// instantiation — this is what lets `S` mean "the same `S`" on both
    /// sides of a use like `in: S # List(S) -> Bool`. Unlike the syntax-tree
    /// walk this replaces, there is no separate `Reference`/name-keyed path
    /// any more: every polymorphic template now declares its variable(s)
    /// with a real `type_var` block (see `docs/polymorphism.md`), so `sort`
    /// can only ever contain `Var`, never a name to match by string.
    fn instantiate_scheme(
        &mut self,
        sort: ResolvedSortId,
        type_vars: &mut HashMap<TypeVarId, InferSortId>,
    ) -> InferSortId {
        match self.ctx.sorts.get(sort).clone() {
            ResolvedSort::Var(id) => *type_vars.entry(id).or_insert_with(|| self.unifier.fresh_var()),
            ResolvedSort::Generic { op, subsort } => {
                let subsort = self.instantiate_scheme(subsort, type_vars);
                self.unifier.generic(op, subsort)
            }
            ResolvedSort::Function { domain, range } => {
                let domain = domain
                    .iter()
                    .map(|&sort| self.instantiate_scheme(sort, type_vars))
                    .collect();
                let range = self.instantiate_scheme(range, type_vars);
                self.unifier.function(domain, range)
            }
            // Already ground — no variable can occur any deeper.
            ResolvedSort::Unit | ResolvedSort::Primitive(_) | ResolvedSort::Def(_) => self.unifier.resolved_node(sort),
        }
    }
}

/// A candidate solution: the measure ranks it against other leaves, and the
/// typing is extracted eagerly because backtracking destroys the variable
/// bindings it is read from.
struct Candidate {
    measure: Vec<u8>,
    /// Whether another leaf reached the same measure; an unbeaten duplicate
    /// means the equation is ambiguous.
    duplicate: bool,
    /// `None` when a free variable remained at this leaf, i.e. the sorts were
    /// underdetermined.
    typing: Option<(Vec<ResolvedSortId>, HashMap<ExprId, NameTarget>)>,
}

/// Solves the constraints by ranked backtracking, in generation order rather
/// than kind-grouped: a disjunction tries every overload, a sub-constraint
/// tries equality first and widening second, a literal takes its most specific
/// admissible number sort.
///
/// Each sub and literal constraint contributes one component to the measure
/// (in generation order, earlier constraints most significant — the arguments
/// of an application precede the equation-level join — and `0` best), so
/// solutions compare lexicographically and the minimum is the most specific
/// typing. Disjunctions contribute no component and are enumerated
/// exhaustively, so tied leaves through different overloads are still detected
/// as ambiguity.
struct Solver<'a> {
    sorts: &'a mut SortInterner,
    unifier: &'a mut Unifier,
    constraints: &'a [Constraint],
    expr_sorts: &'a [InferSortId],
    base_names: &'a HashMap<ExprId, NameTarget>,
    /// The disjunct chosen per disjunction on the current branch.
    choices: Vec<(ExprId, NameTarget)>,
    /// The measure components pushed on the current branch.
    measure: Vec<u8>,
    best: Option<Candidate>,
}

impl Solver<'_> {
    /// Solves the constraints from `index` onward; returns whether any leaf
    /// was reached below this point.
    fn solve(&mut self, index: usize) -> bool {
        if self.dominated() {
            return false;
        }
        let Some(constraint) = self.constraints.get(index) else {
            self.leaf();
            return true;
        };
        match constraint {
            Constraint::Disjunction(disjunction) => self.solve_disjunction(disjunction, index),
            Constraint::Sub(sub) => self.solve_sub(sub, index),
            Constraint::Lit(lit) => self.solve_lit(lit, index),
            Constraint::Comprehension(comprehension) => self.solve_comprehension(comprehension, index),
            Constraint::Join(join) => self.solve_join(join, index),
        }
    }

    /// Binds the join `target` to the lattice least-upper-bound of the branch
    /// `sources` and continues solving — the deterministic counterpart of two
    /// greedy `Sub`s to a shared variable (see [Join]). Every source must be
    /// ground and pairwise joinable; otherwise it defers to the per-source
    /// widening of [Self::solve_join_seq], which decides exactly as the
    /// pre-join two-`Sub` encoding did (so the rare underdetermined branches —
    /// e.g. two empty containers — are unchanged).
    fn solve_join(&mut self, join: &Join, index: usize) -> bool {
        let resolved: Option<Vec<ResolvedSortId>> = join
            .sources
            .iter()
            .map(|&source| self.unifier.resolve(self.sorts, source))
            .collect();
        let Some(resolved) = resolved else {
            return self.solve_join_seq(&join.sources, join.target, 0, index);
        };

        let mut lub = resolved[0];
        for &next in &resolved[1..] {
            match self.sorts.join(lub, next) {
                Some(joined) => lub = joined,
                // Not joinable by the simple lattice (e.g. unequal element
                // sorts): the per-source widening below decides the case the
                // same way the two `Sub`s used to.
                None => return self.solve_join_seq(&join.sources, join.target, 0, index),
            }
        }

        let lub_node = self.unifier.resolved_node(lub);
        if !self.unifier.unify(self.sorts, join.target, lub_node) {
            return false;
        }

        // One widening-distance measure component per source (0 for an exact
        // branch, the number of widening steps otherwise), matching the
        // `solve_sub` convention so the ranking is identical to the two-`Sub`
        // form.
        for &source in &resolved {
            self.measure.push(self.join_distance(source, lub));
        }
        let found = self.solve(index + 1);
        for _ in &resolved {
            self.measure.pop();
        }
        found
    }

    /// The number of lattice widening steps from `source` up to `target`
    /// (`source` a subsort of `target`), the measure a `Sub(source, target)`
    /// would contribute: number-sort generality difference, or one step for a
    /// finite-to-infinite container widening (`FSet` → `Set`, `FBag` → `Bag`).
    fn join_distance(&self, source: ResolvedSortId, target: ResolvedSortId) -> u8 {
        if source == target {
            return 0;
        }
        match (self.sorts.get(source), self.sorts.get(target)) {
            (ResolvedSort::Primitive(source), ResolvedSort::Primitive(target)) => {
                match (number_generality(*source), number_generality(*target)) {
                    (Some(source), Some(target)) if target >= source => (target - source) as u8,
                    _ => 1,
                }
            }
            // A single container-head step; the element sorts are equal (the
            // lattice join keeps them so).
            _ => 1,
        }
    }

    /// The fallback of [Self::solve_join] for underdetermined or non-joinable
    /// branches: widens each source to the shared `target` in turn, exactly as
    /// the two independent `Sub` constraints did before the join fast path
    /// (equality first, then widenings in ascending distance).
    fn solve_join_seq(&mut self, sources: &[InferSortId], target: InferSortId, i: usize, index: usize) -> bool {
        let Some(&source) = sources.get(i) else {
            return self.solve(index + 1);
        };

        // Equality first: it ranks strictly better than any widening.
        let snapshot = self.unifier.snapshot();
        let mut found = false;
        if self.unifier.unify(self.sorts, source, target) {
            self.measure.push(0);
            found = self.solve_join_seq(sources, target, i + 1, index);
            self.measure.pop();
        }
        self.unifier.rollback_to(snapshot);
        if found {
            return true;
        }

        // Then the strict widenings, nearest first (a concrete source upcast,
        // or a concrete target met from below).
        let pairs: Vec<(InferSortId, InferSortId)> =
            if let Some(supers) = self.unifier.strict_super_sorts(self.sorts, source) {
                supers.into_iter().map(|wider| (wider, target)).collect()
            } else if let Some(subsorts) = self.unifier.strict_sub_sorts(self.sorts, target) {
                subsorts.into_iter().map(|narrower| (source, narrower)).collect()
            } else {
                return false;
            };
        for (distance, (lhs, rhs)) in pairs.into_iter().enumerate() {
            let snapshot = self.unifier.snapshot();
            let mut found = false;
            if self.unifier.unify(self.sorts, lhs, rhs) {
                self.measure.push(1 + distance as u8);
                found = self.solve_join_seq(sources, target, i + 1, index);
                self.measure.pop();
            }
            self.unifier.rollback_to(snapshot);
            if found {
                return true;
            }
        }
        false
    }

    /// Branch-and-bound pruning: whether the measure accumulated so far is
    /// already strictly worse, component for component, than the incumbent's
    /// corresponding prefix. A `Disjunction`/`Comprehension` contributes no
    /// measure component of its own (every disjunct is tried, so a tie is
    /// still detected as ambiguity), so without this check every disjunct is
    /// explored to its leaf even once a strictly better solution is already
    /// known — on an equation with many independent overloaded operators
    /// (repeated arithmetic sub-expressions, say) that is exponential in the
    /// number of disjunctions. Pruning is exact: a prefix that is already
    /// strictly greater can never become equal or smaller, since earlier
    /// measure components dominate the lexicographic order, so this changes
    /// nothing about which typing wins or which equations are ambiguous.
    fn dominated(&self) -> bool {
        match &self.best {
            Some(best) => self.measure.as_slice() > &best.measure[..self.measure.len()],
            None => false,
        }
    }

    /// Commits to one disjunct and solves the remaining constraints; all
    /// disjuncts are explored so equal-measure leaves surface as ambiguity.
    fn solve_disjunction(&mut self, disjunction: &Disjunction, index: usize) -> bool {
        let mut found = false;
        for (target, sort) in &disjunction.disjuncts {
            let snapshot = self.unifier.snapshot();
            if self.unifier.unify(self.sorts, disjunction.sort, *sort) {
                trace!(
                    "solver: committing expression {:?} to disjunct {target:?}",
                    disjunction.expr
                );
                self.choices.push((disjunction.expr, *target));
                found |= self.solve(index + 1);
                self.choices.pop();
            }
            self.unifier.rollback_to(snapshot);
        }
        found
    }

    /// Commits to one reading of a set/bag comprehension and solves the rest:
    /// a `Bool` body makes a `Set`, a `Nat` or `Pos` body makes a `Bag`. Like a
    /// disjunction it contributes no measure component and explores every
    /// reading, so an equation where two readings both type (an overloaded body
    /// that can be either boolean or numeric) surfaces as ambiguity.
    fn solve_comprehension(&mut self, comprehension: &Comprehension, index: usize) -> bool {
        let readings = [
            (self.sorts.bool_sort(), ComplexSort::Set),
            (self.sorts.nat_sort(), ComplexSort::Bag),
            (self.sorts.pos_sort(), ComplexSort::Bag),
        ];

        let mut found = false;
        for (body, op) in readings {
            let container = self.sorts.generic(op, comprehension.element);
            let snapshot = self.unifier.snapshot();
            let body_node = self.unifier.resolved_node(body);
            let container_node = self.unifier.resolved_node(container);
            if self.unifier.unify(self.sorts, comprehension.body, body_node)
                && self.unifier.unify(self.sorts, comprehension.node, container_node)
            {
                found |= self.solve(index + 1);
            }
            self.unifier.rollback_to(snapshot);
        }
        found
    }

    fn solve_sub(&mut self, sub: &SubConstraint, index: usize) -> bool {
        // Equality first: it ranks strictly better than any widening, so when
        // it admits a solution the widening choices cannot improve on it and
        // are not explored.
        let snapshot = self.unifier.snapshot();
        let mut found = false;
        if self.unifier.unify(self.sorts, sub.lhs, sub.rhs) {
            self.measure.push(0);
            found = self.solve(index + 1);
            self.measure.pop();
        }
        self.unifier.rollback_to(snapshot);
        if found {
            return true;
        }

        // Otherwise enumerate the strict widenings: a concrete lhs may be
        // upcast, or a concrete rhs met from below. Two unbound variables
        // admit no enumeration and fail.
        let pairs: Vec<(InferSortId, InferSortId)> =
            if let Some(supers) = self.unifier.strict_super_sorts(self.sorts, sub.lhs) {
                supers.into_iter().map(|wider| (wider, sub.rhs)).collect()
            } else if let Some(subsorts) = self.unifier.strict_sub_sorts(self.sorts, sub.rhs) {
                subsorts.into_iter().map(|narrower| (sub.lhs, narrower)).collect()
            } else {
                return false;
            };

        // Widenings rank by distance (ranking them all equally would misreport
        // e.g. a `Pos` argument to `mod` as ambiguous between its `Nat` and
        // `Int` overloads; the minimal upcast is taken instead). The pairs
        // are ordered nearest first, so the first success is the best this
        // constraint can contribute and the rest need not be explored.
        for (distance, (lhs, rhs)) in pairs.into_iter().enumerate() {
            let snapshot = self.unifier.snapshot();
            let mut found = false;
            if self.unifier.unify(self.sorts, lhs, rhs) {
                self.measure.push(1 + distance as u8);
                found = self.solve(index + 1);
                self.measure.pop();
            }
            self.unifier.rollback_to(snapshot);
            if found {
                return true;
            }
        }
        false
    }

    fn solve_lit(&mut self, lit: &LitConstraint, index: usize) -> bool {
        match self.unifier.head(lit.sort) {
            InferSort::Resolved(resolved) => {
                let ResolvedSort::Primitive(sort) = *self.sorts.get(resolved) else {
                    return false;
                };
                let Some(generality) = number_generality(sort) else {
                    return false;
                };
                // `0` is not positive, so a natural literal cannot be `Pos`.
                if lit.kind == LitKind::Natural && sort == Sort::Pos {
                    return false;
                }
                self.measure.push(generality as u8);
                let found = self.solve(index + 1);
                self.measure.pop();
                found
            }
            InferSort::Var(_) => {
                let candidates: &[Sort] = match lit.kind {
                    LitKind::Positive => &[Sort::Pos, Sort::Nat, Sort::Int, Sort::Real],
                    LitKind::Natural => &[Sort::Nat, Sort::Int, Sort::Real],
                };
                // The candidates are ordered most specific first, so the first
                // success is the best this constraint can contribute and the
                // rest need not be explored. The component is the sort's
                // generality, matching the bound branch above, so a literal
                // bound in one branch and free in another still compares
                // consistently.
                for &sort in candidates {
                    let snapshot = self.unifier.snapshot();
                    let resolved = self.sorts.primitive(sort);
                    let node = self.unifier.resolved_node(resolved);
                    let unified = self.unifier.unify(self.sorts, lit.sort, node);
                    debug_assert!(unified, "an unbound variable unifies with any sort");
                    let generality = number_generality(sort).expect("the candidates are number sorts");
                    self.measure.push(generality as u8);
                    let found = self.solve(index + 1);
                    self.measure.pop();
                    self.unifier.rollback_to(snapshot);
                    if found {
                        return true;
                    }
                }
                false
            }
            InferSort::Generic { .. } | InferSort::Function { .. } => false,
        }
    }

    /// A full assignment: keep it when it beats the incumbent, flag a
    /// duplicate when it ties (ambiguity unless later beaten).
    fn leaf(&mut self) {
        debug_assert_eq!(
            self.measure.len(),
            self.constraints
                .iter()
                .map(|constraint| match constraint {
                    Constraint::Sub(_) | Constraint::Lit(_) => 1,
                    // A join contributes one widening component per branch.
                    Constraint::Join(join) => join.sources.len(),
                    Constraint::Disjunction(_) | Constraint::Comprehension(_) => 0,
                })
                .sum::<usize>(),
            "every sub, literal and join-branch contributes exactly one measure component"
        );

        let ordering = match &self.best {
            None => Ordering::Less,
            Some(best) => self.measure.cmp(&best.measure),
        };
        match ordering {
            Ordering::Less => {
                trace!("solver: new best solution at measure {:?}", self.measure);
                let candidate = self.extract();
                self.best = Some(candidate);
            }
            Ordering::Equal => {
                trace!("solver: tie at measure {:?}, ambiguous unless beaten", self.measure);
                self.best.as_mut().expect("a tie requires an incumbent").duplicate = true;
            }
            Ordering::Greater => {}
        }
    }

    /// Reads the solution out of the current variable bindings, before
    /// backtracking destroys them.
    ///
    /// Any sort variable that is still free after solving (e.g. the element
    /// sort of `#[]` where only the container length is observed, never the
    /// element) defaults to `Bool`. This accepts such equations rather than
    /// raising a spurious `UnderdeterminedSort` error.
    fn extract(&mut self) -> Candidate {
        let bool_sort = self.sorts.bool_sort();
        let sorts: Vec<ResolvedSortId> = self
            .expr_sorts
            .iter()
            .map(|&node| self.unifier.resolve_or_default(self.sorts, node, bool_sort))
            .collect();

        let mut names = self.base_names.clone();
        for &(expr, target) in &self.choices {
            names.insert(expr, target);
        }
        Candidate {
            measure: self.measure.clone(),
            duplicate: false,
            typing: Some((sorts, names)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use merc_syntax::ComplexSort;
    use merc_syntax::EqnSpecId;
    use merc_syntax::EquationId;
    use merc_syntax::UntypedDataSpecification;

    use crate::DataSpecification;
    use crate::EquationTyping;
    use crate::ExprId;
    use crate::InferenceError;
    use crate::NameTarget;
    use crate::ResolvedSort;
    use crate::ResolvedSortId;
    use crate::WellTypedError;

    fn typed(text: &str) -> DataSpecification {
        DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap())
            .unwrap_or_else(|err| panic!("expected {text} to typecheck, got {err}"))
    }

    fn inference_error(text: &str) -> InferenceError {
        match DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()) {
            Err(WellTypedError::Inference(error)) => error,
            Err(other) => panic!("expected an inference error for {text}, got {other}"),
            Ok(_) => panic!("expected {text} to be rejected"),
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_infers_basic_equation() {
        let spec = typed("map f: Nat -> Bool; var n: Nat; eqn f(n) = true;");

        // Ids: 0 = `f(n)`, 1 = `n` (arguments before the function), 2 = `f`,
        // 3 = `true`.
        let EquationTyping { sorts, names, .. } = spec.equation_typing((EqnSpecId::new(0), EquationId::new(0)));
        let interner = &spec.context().sorts;
        assert_eq!(sorts[0], interner.bool_sort());
        assert_eq!(sorts[1], interner.nat_sort());
        assert_eq!(sorts[3], interner.bool_sort());
        assert_eq!(names[&ExprId::new(1)], NameTarget::Variable);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_equation_typing_records_spans_and_identifier_names_for_user_equations() {
        let text = "map f: Nat -> Bool; var n: Nat; eqn f(n) = true;";
        let spec = typed(text);

        // Same node numbering as `test_infers_basic_equation`: 0 = `f(n)`,
        // 1 = `n`, 2 = `f`, 3 = `true`.
        let EquationTyping {
            sorts,
            spans,
            identifier_names,
            ..
        } = spec.equation_typing((EqnSpecId::new(0), EquationId::new(0)));

        // `spans` is parallel to `sorts`, one entry per expression node.
        assert_eq!(spans.len(), sorts.len());

        // Each recorded span slices back to the exact source substring it names.
        assert_eq!(&text[spans[1].start..spans[1].end], "n");
        assert_eq!(&text[spans[2].start..spans[2].end], "f");
        assert_eq!(&text[spans[3].start..spans[3].end], "true");
        // Node 0 (`f(n)`, the application) spans the whole call.
        assert_eq!(&text[spans[0].start..spans[0].end], "f(n)");

        // `identifier_names` covers exactly the `Id` nodes (1 and 2 here), keyed the same way as
        // `names`, and independently of what each one resolved to (a variable vs. a mapping).
        assert_eq!(identifier_names[&ExprId::new(1)], "n");
        assert_eq!(identifier_names[&ExprId::new(2)], "f");
        assert_eq!(identifier_names.len(), 2);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_lambda_over_anonymous_struct_is_inferred() {
        // An anonymous binder struct is hoisted, so a construct binding one is
        // typed like any other equation. (There is no longer a "skipped"
        // outcome: every equation that type checks is fully inferred.)
        let spec = typed("map f: (struct t) -> Bool; g: (struct t) -> Bool; eqn g = lambda x: struct t. f(x);");
        let EquationTyping { .. } = spec.equation_typing((EqnSpecId::new(0), EquationId::new(0)));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_numeric_literals_stay_positive() {
        let spec = typed("map p: Pos; eqn p = 1 + 2;");

        // Ids: 0 = `p`, 1 = `+(1, 2)`, 2 = `1`, 3 = `2`, 4 = `+`.
        let EquationTyping { sorts, .. } = spec.equation_typing((EqnSpecId::new(0), EquationId::new(0)));
        let interner = &spec.context().sorts;
        assert_eq!(sorts[1], interner.pos_sort());
        assert_eq!(sorts[2], interner.pos_sort());
        assert_eq!(sorts[3], interner.pos_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_literal_argument_keeps_minimal_sort_under_upcast() {
        let spec = typed("map f: Int -> Bool; b: Bool; eqn b = f(1);");

        // Ids: 0 = `b`, 1 = `f(1)`, 2 = `1`, 3 = `f`. The literal keeps its
        // minimal sort; Phase-4 lowering inserts the upcast to `Int`.
        let EquationTyping { sorts, .. } = spec.equation_typing((EqnSpecId::new(0), EquationId::new(0)));
        let interner = &spec.context().sorts;
        assert_eq!(sorts[1], interner.bool_sort());
        assert_eq!(sorts[2], interner.pos_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_equality_scheme_on_user_sort() {
        let spec = typed("sort D; cons d: D; map b: Bool; eqn b = d == d;");

        // Ids: 0 = `b`, 1 = `==(d, d)`, 2/3 = `d`, 4 = `==`.
        let EquationTyping { sorts, names, .. } = spec.equation_typing((EqnSpecId::new(0), EquationId::new(0)));
        assert_eq!(names[&ExprId::new(4)], NameTarget::Builtin);
        assert_eq!(sorts[1], spec.context().sorts.bool_sort());
        assert_eq!(sorts[2], spec.sort_of_constructor(merc_syntax::ConstructorId::new(0)));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_if_scheme_infers_element_sort() {
        let spec = typed("map n: Nat; eqn n = if(true, 1, 2);");

        // Ids: 0 = `n`, 1 = the application, 2 = `true`, 3 = `1`, 4 = `2`,
        // 5 = `if`. The branches stay `Pos`; the join upcasts to `Nat`.
        let EquationTyping { sorts, names, .. } = spec.equation_typing((EqnSpecId::new(0), EquationId::new(0)));
        assert_eq!(names[&ExprId::new(5)], NameTarget::Builtin);
        assert_eq!(sorts[1], spec.context().sorts.pos_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_free_element_sort_defaults_to_bool() {
        // A free element sort (the element of an empty list whose sort is
        // never constrained by context) defaults to Bool rather than causing
        // UnderdeterminedSort.
        let spec = typed("map b: Bool; eqn b = [] == [];");
        // ExprIds: 0 = `b`, 1 = `==([], [])`, 2 = first `[]`, 3 = second
        // `[]`, 4 = `==`. Both empty lists take List(Bool).
        let (sorts, _) = typing(&spec);
        let interner = &spec.context().sorts;
        for &list_sort in &sorts[2..=3] {
            let ResolvedSort::Generic { op, subsort } = interner.get(list_sort) else {
                panic!("expected a container sort");
            };
            assert_eq!(*op, ComplexSort::List);
            assert_eq!(*subsort, interner.bool_sort());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow
    fn test_undeclared_name() {
        let text = "map b: Bool; eqn b = undeclared;";
        let error = inference_error(text);
        match &error {
            InferenceError::UndeclaredName { name, span } => {
                assert_eq!(name, "undeclared");
                assert_eq!(&text[span.start..span.end], "undeclared");
            }
            other => panic!("expected UndeclaredName, got {other}"),
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_incompatible_sides_have_no_typing() {
        let text = "map f: Bool; eqn f = 1;";
        let error = inference_error(text);
        match &error {
            InferenceError::NoTyping { expression, sort, span } => {
                // The whole equation (including its trailing `;`) is the
                // offending unit; nothing narrower pins down a sort to blame.
                assert_eq!(&text[span.start..span.end], "f = 1;");
                assert_eq!(expression, "f = 1");
                // No externally-supplied expected sort: this is a whole-equation check
                // (`Roots::Equation`), not a `check_expression_against` call.
                assert_eq!(sort, &None);
            }
            other => panic!("expected NoTyping, got {other}"),
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_non_boolean_condition() {
        let text = "map f: Nat -> Bool; var n: Nat; eqn n -> f(n) = true;";
        let error = inference_error(text);
        match &error {
            InferenceError::ConditionNotBool { condition, span } => {
                assert_eq!(condition, "n");
                // The span points at the condition `n`, not the variable
                // declaration earlier in the text.
                assert_eq!(&text[span.start..span.end], "n");
                assert_eq!(span.start, text.rfind("n ->").expect("condition is present"));
            }
            other => panic!("expected ConditionNotBool, got {other}"),
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow
    fn test_sides_join_through_upcast() {
        let spec = typed("map f: Nat; eqn f = 1;");

        // Ids: 0 = `f`, 1 = `1`. The literal stays `Pos` and is upcast into
        // the join with the `Nat` left-hand side.
        let EquationTyping { sorts, .. } = spec.equation_typing((EqnSpecId::new(0), EquationId::new(0)));
        let interner = &spec.context().sorts;
        assert_eq!(sorts[0], interner.nat_sort());
        assert_eq!(sorts[1], interner.pos_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_lambda_infers_function_sort() {
        let spec = typed("map f: Nat -> Bool; eqn f = lambda n: Nat. true;");

        // Ids: 0 = `f`, 1 = `lambda n: Nat. true`, 2 = `true`. The lambda's
        // own sort is the function from its bound variable's declared sort to
        // its body's sort; the bound variable `n` has no id of its own.
        let (sorts, _) = typing(&spec);
        let interner = &spec.context().sorts;
        assert_eq!(sorts[0], spec.sort_of_map(merc_syntax::MapId::new(0)));
        match interner.get(sorts[1]) {
            ResolvedSort::Function { domain, range } => {
                assert_eq!(domain.as_slice(), [interner.nat_sort()]);
                assert_eq!(*range, interner.bool_sort());
            }
            other => panic!("expected a function sort, got {other:?}"),
        }
        assert_eq!(sorts[2], interner.bool_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_quantifier_infers_bool_sort() {
        let spec = typed("map b: Bool; eqn b = forall n: Nat. n >= 0;");

        // A `forall`/`exists` is always `Bool`, regardless of the body.
        let (sorts, _) = typing(&spec);
        let interner = &spec.context().sorts;
        assert_eq!(sorts[0], interner.bool_sort());
        assert_eq!(sorts[1], interner.bool_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_quantifier_requires_boolean_body() {
        let text = "map b: Bool; eqn b = forall n: Nat. n;";
        let error = inference_error(text);
        match &error {
            InferenceError::QuantifierNotBool { body, span } => {
                assert_eq!(body, "n");
                // The span points at the body `n`, not the bound variable
                // declaration `n: Nat` just before it.
                assert_eq!(&text[span.start..span.end], "n");
                assert_eq!(span.start, text.len() - 2, "the body is the last token before ';'");
            }
            other => panic!("expected QuantifierNotBool, got {other}"),
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_where_binds_variable_to_assignment_sort() {
        // `x` inside the body takes the sort inferred for its assignment `2`
        // (here upcast to `Nat`, matching `g`'s declared sort), rather than a
        // declared binder sort.
        let spec = typed("map g: Nat; eqn g = (x + 1) whr x = 2 end;");
        let EquationTyping { .. } = spec.equation_typing((EqnSpecId::new(0), EquationId::new(0)));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_where_assignments_do_not_see_each_other() {
        // Every assignment's right-hand side is typed against the outer
        // scope, not against sibling bindings, so `y`'s `x` resolves to the
        // declared `Nat` variable even though this `whr` also rebinds `x` to
        // a `Bool` (every assignment is typed against the original declared
        // variables, the context being extended once, for the body).
        let spec = typed("map f: Nat -> Bool; var x: Nat; eqn f(x) = true whr x = false, y = x end;");

        // Ids: 0 = `f(x)`, 1 = `x`, 2 = `f`, 3 = the `whr` expression,
        // 4 = `false` (the `x` assignment), 5 = `x` (the `y` assignment,
        // resolved before either name is shadowed), 6 = `true` (the body).
        let (sorts, _) = typing(&spec);
        let interner = &spec.context().sorts;
        assert_eq!(sorts[5], interner.nat_sort());
    }

    /// Extracts the inferred sorts and name targets of the first equation.
    fn typing(spec: &DataSpecification) -> (&[ResolvedSortId], &HashMap<ExprId, NameTarget>) {
        let EquationTyping { sorts, names, .. } = spec.equation_typing((EqnSpecId::new(0), EquationId::new(0)));
        (sorts, names)
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow
    fn test_set_literal_takes_finite_set_sort() {
        let spec = typed("map s: FSet(Pos); eqn s = {1, 2};");

        // Ids: 0 = `s`, 1 = `{1, 2}`, 2 = `1`, 3 = `2`. The literal's sort is
        // the declared `FSet(Pos)`.
        let (sorts, _) = typing(&spec);
        assert_eq!(sorts[1], spec.sort_of_map(merc_syntax::MapId::new(0)));
        assert_eq!(sorts[2], spec.context().sorts.pos_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow
    fn test_set_literal_widens_to_set_at_use() {
        let spec = typed("map s: Set(Nat); eqn s = {1, 2};");

        // Ids: 0 = `s`, 1 = `{1, 2}`, 2/3 = the literals. The enumeration
        // stays `FSet(Nat)` — Phase-4 lowering materializes the widening to
        // the `Set(Nat)` join — and the literals keep their minimal `Pos`.
        let (sorts, _) = typing(&spec);
        let interner = &spec.context().sorts;
        let ResolvedSort::Generic { op, subsort } = interner.get(sorts[1]) else {
            panic!("expected a container sort");
        };
        assert_eq!(*op, ComplexSort::FSet);
        assert_eq!(*subsort, interner.nat_sort());
        assert_eq!(sorts[2], interner.pos_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow
    fn test_set_elements_join_to_common_supersort() {
        let spec = typed("map s: Int -> FSet(Int); var n: Int; eqn s(n) = {1, n};");

        // Ids: 0 = `s` (the applied function symbol), 1 = `n` (the lhs argument), 2 = `s(n)` (the
        // whole application, the declared map sort), 3 = the rhs set, 4 = `1`, 5 = `n` (the rhs
        // occurrence). The element sort is the join `Int`; the literal itself stays `Pos`.
        let (sorts, _) = typing(&spec);
        assert_eq!(sorts[2], spec.sort_of_map(merc_syntax::MapId::new(0)));
        assert_eq!(sorts[4], spec.context().sorts.pos_sort());
        assert_eq!(sorts[5], spec.context().sorts.int_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow
    fn test_incompatible_set_elements_have_no_typing() {
        let error = inference_error("map s: FSet(Nat); eqn s = {1, true};");
        assert!(matches!(error, InferenceError::NoTyping { .. }), "{error}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_empty_set_takes_element_sort_from_context() {
        let spec = typed("map s: Set(Nat); eqn s = {};");

        // Ids: 0 = `s`, 1 = `{}`, typed `FSet(Nat)` under the `Set(Nat)` join.
        let (sorts, _) = typing(&spec);
        let interner = &spec.context().sorts;
        let ResolvedSort::Generic { op, subsort } = interner.get(sorts[1]) else {
            panic!("expected a container sort");
        };
        assert_eq!(*op, ComplexSort::FSet);
        assert_eq!(*subsort, interner.nat_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_free_empty_set_defaults_to_bool() {
        // Same as test_free_element_sort_defaults_to_bool: a free element sort
        // of an empty finite set defaults to Bool.
        let spec = typed("map b: Bool; eqn b = {} == {};");
        // ExprIds: 0 = `b`, 1 = `==([], [])`, 2 = first `{}`, 3 = second
        // `{}`, 4 = `==`. Both empty sets take FSet(Bool).
        let (sorts, _) = typing(&spec);
        let interner = &spec.context().sorts;
        for &set_sort in &sorts[2..=3] {
            let ResolvedSort::Generic { op, subsort } = interner.get(set_sort) else {
                panic!("expected a container sort");
            };
            assert_eq!(*op, ComplexSort::FSet);
            assert_eq!(*subsort, interner.bool_sort());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_bag_literal_counts_are_natural() {
        let spec = typed("map b: FBag(Nat); eqn b = {0: 2};");

        // Ids: 0 = `b`, 1 = the bag, 2 = `0`, 3 = the count `2`. The count
        // keeps its minimal `Pos` and is upcast into the `Nat` it must have.
        let (sorts, _) = typing(&spec);
        assert_eq!(sorts[1], spec.sort_of_map(merc_syntax::MapId::new(0)));
        assert_eq!(sorts[2], spec.context().sorts.nat_sort());
        assert_eq!(sorts[3], spec.context().sorts.pos_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_bag_count_must_be_a_natural_number() {
        let error = inference_error("map b: FBag(Nat); r: Real; eqn b = {0: r};");
        assert!(matches!(error, InferenceError::NoTyping { .. }), "{error}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_empty_bag_takes_element_sort_from_context() {
        let spec = typed("map b: Bag(Pos); eqn b = {:};");

        let (sorts, _) = typing(&spec);
        let interner = &spec.context().sorts;
        let ResolvedSort::Generic { op, subsort } = interner.get(sorts[1]) else {
            panic!("expected a container sort");
        };
        assert_eq!(*op, ComplexSort::FBag);
        assert_eq!(*subsort, interner.pos_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_set_comprehension_from_boolean_body() {
        let spec = typed("map s: Set(Nat); eqn s = { n: Nat | n < 3 };");

        // Ids: 0 = `s`, 1 = the comprehension, 2 = `<(n, 3)`, 3 = `n`,
        // 4 = `3`, 5 = `<`. The boolean body makes a `Set(Nat)`; the bound
        // variable resolves like an equation variable.
        let (sorts, names) = typing(&spec);
        assert_eq!(sorts[1], spec.sort_of_map(merc_syntax::MapId::new(0)));
        assert_eq!(sorts[2], spec.context().sorts.bool_sort());
        assert_eq!(sorts[3], spec.context().sorts.nat_sort());
        assert_eq!(names[&ExprId::new(3)], NameTarget::Variable);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_bag_comprehension_from_numeric_body() {
        let spec = typed("map b: Pos -> Bag(Nat); var m: Pos; eqn b(m) = { n: Nat | m };");

        // Ids: 0 = `b(m)` (the application), 1 = `m` (the lhs argument), 2 = `b`
        // (the applied function symbol, the declared map sort), 3 = the rhs
        // comprehension, 4 = `m` (the rhs occurrence, the `Nat` body reads as the
        // multiplicity function of the `Bag(Nat)`). As in the set-literal case,
        // the leaf occurrence itself stays `Pos`; only the aggregate sorts are
        // joined to `Bag(Nat)`.
        let (sorts, _) = typing(&spec);
        assert_eq!(sorts[2], spec.sort_of_map(merc_syntax::MapId::new(0)));
        assert_eq!(sorts[1], spec.context().sorts.pos_sort());
        assert_eq!(sorts[4], spec.context().sorts.pos_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_bag_comprehension_from_positive_body() {
        let spec = typed("map b: Bag(Pos); eqn b = { p: Pos | 2 };");

        // The `Pos` body also reads as a bag; the body keeps its minimal sort
        // and Phase-4 lowering inserts the `Pos` → `Nat` coercion, as mCRL2
        // does.
        let (sorts, _) = typing(&spec);
        assert_eq!(sorts[1], spec.sort_of_map(merc_syntax::MapId::new(0)));
        assert_eq!(sorts[2], spec.context().sorts.pos_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_comprehension_body_must_be_bool_or_number() {
        let error = inference_error("map r: Real; s: Set(Nat); eqn s = { n: Nat | r };");
        assert!(matches!(error, InferenceError::NoTyping { .. }), "{error}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_comprehension_readings_can_be_ambiguous() {
        // `f(n)` types as both `Bool` (a set) and `Nat` (a bag), and `==`
        // accepts either pair, so the equation is genuinely ambiguous.
        let error = inference_error(
            "map f: Nat -> Bool; f: Nat -> Nat; b: Bool; eqn b = { n: Nat | f(n) } == { n: Nat | f(n) };",
        );
        assert!(matches!(error, InferenceError::AmbiguousExpression { .. }), "{error}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_comprehension_variable_shadows_declarations() {
        // The bound `n: Nat` shadows the boolean map `n` inside the predicate
        // and stops shadowing it after the comprehension.
        let spec = typed("map n: Bool; s: Set(Nat); b: Bool; eqn b = ({ n: Nat | n < 3 } == s) && n;");

        let EquationTyping { .. } = spec.equation_typing((EqnSpecId::new(0), EquationId::new(0)));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_comprehension_over_alias_and_user_sort() {
        let spec = typed("sort A = Nat; map s: Set(A); eqn s = { a: A | a < 3 };");
        let (sorts, _) = typing(&spec);
        assert_eq!(sorts[1], spec.sort_of_map(merc_syntax::MapId::new(0)));

        typed("sort D = struct d1 | d2; map s: Set(D); eqn s = { x: D | x == d1 };");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_product_binder_sort_is_rejected() {
        // A bare product is not a valid variable sort; a binder over one is
        // now rejected rather than left untyped (which previously let an
        // ill-typed body slip through unchecked).
        let text = "map s: Set(Nat); eqn s = { x: Nat # Nat | true };";
        let err = inference_error(text);
        match &err {
            InferenceError::InvalidBinderSort { sort, span, .. } => {
                // `Display` parenthesizes the product sort; the span still
                // points at the unparenthesized source text.
                assert_eq!(sort, "(Nat # Nat)");
                // The span is precisely the binder's own identifier.
                assert_eq!(&text[span.start..span.end], "x");
            }
            other => panic!("expected InvalidBinderSort, got {other}"),
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_polymorphic_membership_over_undeclared_container_sort() {
        // No container sort occurs in any declaration, so `in` exists only
        // through the polymorphic template signature (the mCRL2 corpus uses
        // this pattern heavily: `x in { a, b }` over an enumerated sort).
        let spec = typed("sort D = struct a | b | c; map f: D -> Bool; var x: D; eqn f(x) = x in { a, b };");

        // Ids: 0 = `f(x)`, 1 = `x`, 2 = `f`, 3 = `in(x, {a, b})`, 4 = `x`,
        // 5 = `{a, b}`, 6 = `a`, 7 = `b`, 8 = `in`.
        let (sorts, names) = typing(&spec);
        let interner = &spec.context().sorts;
        let ResolvedSort::Generic { op, subsort } = interner.get(sorts[5]) else {
            panic!("expected a container sort");
        };
        assert_eq!(*op, ComplexSort::FSet);
        assert_eq!(*subsort, spec.sort_of_constructor(merc_syntax::ConstructorId::new(0)));
        assert_eq!(names[&ExprId::new(8)], NameTarget::Builtin);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_polymorphic_list_operations() {
        // `head` and `|>` come from the list template; no `List` sort is
        // declared anywhere.
        let spec = typed("map n: Nat; eqn n = head([1, 2]);");
        let (sorts, _) = typing(&spec);
        // Ids: 0 = `n`, 1 = `head([1, 2])`. The list elements stay `Pos` and
        // the result is upcast into the `Nat` join.
        assert_eq!(sorts[1], spec.context().sorts.pos_sort());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_function_update_is_polymorphic() {
        let spec = typed("map f: Nat -> Bool; g: Nat -> Bool; eqn g = f[1 -> true];");

        // Ids: 0 = `g`, 1 = the update, 2 = `f`, 3 = `1`, 4 = `true`,
        // 5 = `@func_update`.
        let (sorts, names) = typing(&spec);
        assert_eq!(sorts[1], spec.sort_of_map(merc_syntax::MapId::new(0)));
        assert_eq!(names[&ExprId::new(5)], NameTarget::Builtin);
    }
}
