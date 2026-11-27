use std::convert::Infallible;
use std::fmt;

use log::debug;

use merc_collections::IndexedSet;
use merc_syntax::DataExpr;
use merc_syntax::EqnSpecId;
use merc_syntax::EquationId;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::SortId;
use merc_syntax::SourceMap;
use merc_syntax::Traverse;
use merc_syntax::UntypedDataSpecification;

use crate::AliasError;
use crate::EquationTyping;
use crate::InferenceError;
use crate::NumberEncoding;
use crate::Roots;
use crate::TypeCheckContext;
use crate::TypedRoots;
use crate::TypingInfo;
use crate::WellTypedError;
use crate::apply_sorts_in_data_expr;
use crate::apply_sorts_in_spec;
use crate::assign_declaration_ids;
use crate::basic_sort_data_specification;
use crate::build_signature;
use crate::check_aliases;
use crate::check_equation_well_formedness;
use crate::check_no_system_function_redeclaration;
use crate::check_products_within_domains;
use crate::check_system_specification;
use crate::desugar_structured_sorts;
use crate::function_update_arities;
use crate::hoist_anonymous_structs;
use crate::infer_expression;
use crate::is_basic_sort_name;
use crate::is_well_typed;
use crate::lower_data_expr;
use crate::lower_data_expressions;
use crate::normalize_sorts;
use crate::resolve_data_expr_variables;
use crate::resolve_data_specification_variables;
use crate::resolve_sort;
use crate::resolve_sort_id;
use crate::resolve_sort_ids;
use crate::resolve_system_signature;
use crate::resolve_type_variables;
use crate::structured_sort_equations;
use crate::typecheck_equations;
use crate::typecheck_function_update_template;
use crate::typecheck_system_equations;
use crate::typecheck_templates;
use crate::typing_info;

/// A type checked and well-typed data specification.
///
/// Holds the resolved user declarations, the sort-name → [`SortId`] map assigned
/// during name resolution, and the system-defined (Appendix-B) declarations for
/// the sorts that occur.
pub struct DataSpecification {
    spec: UntypedDataSpecification,
    sorts: IndexedSet<String>,

    system: UntypedDataSpecification,
    context: TypeCheckContext,

    encoding: NumberEncoding,
    /// Every sort-name reference in `spec`'s own declarations.
    sort_references: Vec<typing_info::SortReference>,
    /// The reverse lookup from a name occurrence to its declaration.
    declaration_index: typing_info::DeclarationIndex,
}

impl DataSpecification {
    /// Type checks `spec` against a fresh, throwaway [`SourceMap`], using the default number
    /// encoding.
    ///
    /// Prefer [`Self::from_untyped_with`] with a real `sources` when `spec` came from a file on disk
    /// that may itself `%import` other specifications.
    pub fn from_untyped(spec: UntypedDataSpecification) -> Result<Self, WellTypedError> {
        Self::from_untyped_with(spec, NumberEncoding::default(), &mut SourceMap::new())
    }

    /// Create a completed well-typed data specification from an untyped data
    /// specification, using `encoding` to represent the numeric sorts.
    ///
    /// `sources` accumulates the system-defined (Appendix-B) content this generates as virtual
    /// documents, and resolve %import directives correctly.
    pub fn from_untyped_with(
        mut spec: UntypedDataSpecification,
        encoding: NumberEncoding,
        sources: &mut SourceMap,
    ) -> Result<Self, WellTypedError> {
        debug!(
            "starting on {} sort, {} constructor, {} map and {} equation declaration(s)",
            spec.sort_declarations.len(),
            spec.constructor_declarations.len(),
            spec.map_declarations.len(),
            spec.equation_declarations.len()
        );

        // Ties every equation-variable occurrence to its own `var`-block
        // declaration span.
        resolve_data_specification_variables(&mut spec);

        // Hoist anonymous structured sorts into fresh named declarations.
        hoist_anonymous_structs(&mut spec);
        debug!(
            "hoisted anonymous structs; {} sort declaration(s) remain",
            spec.sort_declarations.len()
        );

        apply_sorts_in_spec(&mut spec, |sort| -> Result<_, Infallible> {
            Ok(flatten_function_sorts(sort))
        })
        .expect("The inner function never fails");

        // Assign ids to `type_var` declarations and resolve every reference to one to its id.
        resolve_type_variables(&mut spec)?;
        debug!("resolved type variable name(s)");

        // `system` starts out as just the basic sorts.
        let mut system = basic_sort_data_specification(sources, encoding);

        // This is a bit ugly, but we want to give the system-defined sorts
        // disticts ids in the spec.sort_declarations.
        spec.sort_declarations.extend(
            system
                .sort_declarations
                .iter()
                .filter(|decl| !is_basic_sort_name(&decl.identifier))
                .cloned(),
        );

        // The returned sorts are only used for lookup cycles.
        let sorts = resolve_sort_ids(&mut spec)?;
        debug!("resolved {} sort name(s)", sorts.len());

        // Resolve the same sort names in `system` as were resolved in `spec`.
        apply_sorts_in_spec(&mut system, |sort| resolve_sort_id(sort, &sorts))?;

        // Alias checks still need to see the structured sorts, so we perform them before desugaring.
        check_aliases(&spec).map_err(|(err, span)| {
            let name = |id: &SortId| sorts.get_by_index(**id).expect("The sort should be declared").clone();
            match err {
                AliasError::Circular { cycle } => WellTypedError::AliasCycle {
                    sorts: cycle.iter().map(name).collect(),
                    span,
                },
                AliasError::ThroughFunctionSort { sort } => WellTypedError::RecursiveAliasThroughFunctionSort {
                    sort: name(&sort),
                    span,
                },
            }
        })?;

        // Desugar structured sorts into abstract sorts plus their constructors,
        // recognisers and projections.
        let structs = desugar_structured_sorts(&mut spec);
        debug!(
            "desugared {} structured sort(s) into {} constructor(s)",
            structs.len(),
            structs.iter().map(Vec::len).sum::<usize>()
        );

        // Assign ids to desugared declarations.
        assign_declaration_ids(&mut spec);

        // Compute the (S, C, M) signature and run the signature-layer checks of
        // definition 15.1.7. This runs before alias expansion so the errors refer to sorts
        // as the user wrote them.
        let mut context = TypeCheckContext::new();
        build_signature(&mut context, &spec)?;
        debug!("signature checks passed");

        // Every sort-name reference `spec`'s own declarations make.
        let sort_references = typing_info::collect_data_specification_sort_references(&spec);
        debug!("collected {} sort-name reference(s)", sort_references.len());

        // Expand aliases to a canonical form now that they are known to be
        // acyclic.
        normalize_sorts(&mut spec);
        debug!("normalized alias indirection");

        // Additional well-typedness checks over the normalized spec.
        is_well_typed(&spec)?;
        debug!("well-typedness checks passed");

        // Lower the built-in operator nodes in the user equations to named
        // applications.
        lower_data_expressions(&mut spec);
        debug!("lowered the user equations");

        // The system-defined part of type-checking is deliberately narrow now:
        // `system` holds only the basic sorts so far.
        check_no_system_function_redeclaration(&spec, &system)?;
        debug!("no user declaration redeclares a system function");

        // The defining equations of each structured sort (Appendix B.10) join the system-defined
        // part, checked the same way as every other system equation.
        for constructors in &structs {
            system.merge(&structured_sort_equations(sources, constructors).map_err(WellTypedError::Custom)?);
        }

        // A struct's own equations are generated as fresh source text, and so
        // sort ids must be resolved.
        apply_sorts_in_spec(&mut system, |sort| resolve_sort_id(sort, &sorts))?;

        // The system equations parse with the same operator nodes, so they are
        // lowered like the user equations.
        lower_data_expressions(&mut system);

        debug!(
            "built the base system-defined specification with {} sort, {} map and {} equation \
             declaration(s)",
            system.sort_declarations.len(),
            system.map_declarations.len(),
            system.equation_declarations.len()
        );

        // Resolve `system`'s own declarations.
        resolve_system_signature(&mut context, &spec, &system)?;
        debug!("resolved the system signature");

        debug!(
            "Signature: \n{}",
            context
                .signature
                .as_ref()
                .expect("Signature was constructed at this point")
                .display(&context, &spec)
        );

        // Type checks every container template's own equations once.
        typecheck_templates(&mut context, sources, encoding)?;
        debug!("container template equations passed the rigid check");

        // Every distinct function-update arity `spec` declares gets its own
        // generic template.
        let mut function_update_arities_needed = function_update_arities(&spec);
        function_update_arities_needed.insert(1);
        for arity in function_update_arities_needed {
            typecheck_function_update_template(&mut context, sources, arity)?;
        }
        debug!("function-update template equations passed the rigid check");

        // Inference over every user equation; an equation binding
        // a variable through an invalid sort (a bare product) is rejected here.
        typecheck_equations(&mut context, &spec)?;
        debug!("inference finished; the specification is well-typed");

        // Ties every system equation's own variable occurrences to its `var`-block declaration.
        resolve_data_specification_variables(&mut system);

        // A sanity net over the generated content, cannot use is_well_typed
        // directly.
        check_system_specification(&spec, &system)?;
        check_equation_well_formedness(&system)?;
        debug!(
            "final system-defined specification has {} sort, {} map and {} equation declaration(s)",
            system.sort_declarations.len(),
            system.map_declarations.len(),
            system.equation_declarations.len()
        );

        assign_declaration_ids(&mut system);

        // `system` at this point holds only `basics` and the desugared
        // structs' own equations).
        typecheck_system_equations(&mut context, &spec, &system)?;
        debug!("system-equation inference finished; the system specification is well-typed");

        let declaration_index = typing_info::DeclarationIndex::build(&mut context, &spec, &system);

        Ok(Self {
            spec,
            sorts,
            system,
            context,
            encoding,
            sort_references,
            declaration_index,
        })
    }

    /// The number encoding this specification was built with.
    pub fn number_encoding(&self) -> NumberEncoding {
        self.encoding
    }

    /// The resolved data specification.
    pub fn data_specification(&self) -> &UntypedDataSpecification {
        &self.spec
    }

    /// Maps each declared sort name to the [`SortId`] assigned during name
    /// resolution.
    pub fn sorts(&self) -> &IndexedSet<String> {
        &self.sorts
    }

    /// The system-defined (Appendix-B) declarations for the basic and container
    /// sorts that occur in the specification, plus the defining equations of
    /// the desugared structured sorts (Appendix B.10).
    pub fn system_defined_specification(&self) -> &UntypedDataSpecification {
        &self.system
    }

    /// The reverse lookup from a name occurrence to its declaration.
    pub(crate) fn declaration_index(&self) -> &typing_info::DeclarationIndex {
        &self.declaration_index
    }

    /// The query context holding the sorts interned during type checking,
    /// backing the declaration-sort accessors below.
    pub(crate) fn context(&self) -> &TypeCheckContext {
        &self.context
    }

    /// Typing of the equation identified by `key`, read from the
    /// `equation_typing` cache that `from_untyped` populated. Requires `key` to
    /// index an equation of this specification.
    pub(crate) fn equation_typing(&self, key: (EqnSpecId, EquationId)) -> &EquationTyping {
        self.context
            .equation_typing
            .get(&key)
            .expect("equation typings are all resolved during from_untyped")
            .as_ref()
            .expect("a well-typed specification has no equation inference errors")
    }

    /// Renders `self.data_specification()` the same way its own `Display` does
    /// except each equation's own sub-expressions are annotated with their
    /// resolved sort (`expr:Sort`) rather than left implicit.
    pub fn to_typed_string(&self) -> TypedDataSpecification<'_> {
        TypedDataSpecification { spec: self }
    }

    /// Type checks a single data expression against this specification. The result can be
    /// lowered to the aterm form with `merc_lowering::lower_expression`, so it can be handed
    /// straight to a rewriter built from that specification.
    ///
    /// Takes `&mut self` because inference interns the sorts it discovers into
    /// the shared context; the specification itself is not modified.
    pub fn typecheck_expression(&mut self, expr: &DataExpr) -> Result<TypedExpression, InferenceError> {
        // Ties every occurrence of one of `expr`'s own binders to it.
        let mut expr = expr.clone();
        resolve_data_expr_variables(&mut expr);

        // The built-in operator nodes (`x + y`, `[x, y]`, `f[x -> y]`) become
        // applications first, exactly as `from_untyped_with` does for the
        // equations: inference and lowering both require a lowered expression.
        let expr = lower_data_expr(expr);

        let typing = infer_expression(&mut self.context, &self.spec, &expr)?;
        let info = typing_info::build(self, Roots::Expression(&expr), &typing);
        Ok(TypedExpression { expr, typing, info })
    }

    /// The typing of one user equation, span-keyed so hover/go-to-definition can look up a
    /// sub-expression by source position (see [`TypingInfo::at_offset`]).
    ///
    /// # Panics
    ///
    /// `key` must index an equation of this specification (the `EqnSpecId`/`EquationId` on
    /// [`Self::data_specification`]); panics otherwise.
    pub fn equation_typing_info(&self, key: (EqnSpecId, EquationId)) -> TypingInfo {
        let (eqn_spec_id, equation_id) = key;
        let equation = &self.spec.equation_declarations[*eqn_spec_id].equations[*equation_id];
        typing_info::build(self, Roots::equation(equation), self.equation_typing(key))
    }

    /// Every user equation's typing, merged into one table, in declaration order, plus a
    /// [`crate::ResolvedName::Sort`] node for every sort-name reference this specification's own
    /// declarations make (`cons`/`map` signatures, a `var`-block, a sort alias's own right-hand
    /// side, a binder inside an equation) — see [`Self::equation_typing_info`] for the per-
    /// equation version, which does *not* include these: a sort declaration isn't scoped to any
    /// one equation, so there is nothing meaningful to slice per `key` the way the rest of this
    /// method's result is.
    pub fn typing_info(&self) -> TypingInfo {
        let mut info = TypingInfo::default();
        for eqn_spec in &self.spec.equation_declarations {
            let eqn_spec_id = eqn_spec.id.expect("assign_declaration_ids ran during from_untyped");
            for equation in &eqn_spec.equations {
                let equation_id = equation.id.expect("assign_declaration_ids ran during from_untyped");
                let typing = self.equation_typing((eqn_spec_id, equation_id));
                info.merge(typing_info::build(self, Roots::equation(equation), typing));
            }
        }

        typing_info::push_sort_references(self, &self.sort_references, &mut info);
        info
    }

    /// Resolves a sort expression that occurs *outside* the data specification proper — an
    /// action argument, a process parameter, a global variable declaration — against this
    /// already-resolved specification's declared sort names.
    ///
    /// Requires `sort` to contain no anonymous `struct` (see `merc_process`).
    #[cfg_attr(
        not(feature = "internal"),
        expect(dead_code, reason = "only the `internal` API uses it")
    )]
    pub(crate) fn resolve_declared_sort(
        &mut self,
        sort: &SortExpression,
    ) -> Result<crate::ResolvedSortId, WellTypedError> {
        check_products_within_domains(sort)?;
        let resolved = resolve_sort_id(sort, &self.sorts)?;
        Ok(resolve_sort(&mut self.context, &self.spec, &resolved))
    }

    /// Splits into simultaneous borrows of the query context and the resolved specification, for
    /// a caller (process-level checking, see `merc_process`) that needs to run inference
    /// against a variable scope of its own rather than one of `self`'s own equations.
    #[cfg_attr(
        not(feature = "internal"),
        expect(dead_code, reason = "only the `internal` API uses it")
    )]
    pub(crate) fn context_and_specs_mut(&mut self) -> (&mut TypeCheckContext, &UntypedDataSpecification) {
        (&mut self.context, &self.spec)
    }

    /// Resolves the sort names of every binder (`lambda`/`forall`/`exists`/a comprehension) in
    /// `expr`, in place.
    ///
    /// An equation's own binder sorts are already resolved by `from_untyped_with`'s pipeline. Call
    /// this before checking any expression from *outside* the data specification — a process-body
    /// expression (an action argument, a condition, …), or a caller-supplied expression to
    /// [`Self::typecheck_expression`] — so a binder over a user-declared sort name resolves
    /// correctly.
    #[cfg_attr(
        not(feature = "internal"),
        expect(dead_code, reason = "only the `internal` API uses it")
    )]
    pub(crate) fn resolve_expression_binder_sorts(&mut self, expr: &mut DataExpr) -> Result<(), WellTypedError> {
        apply_sorts_in_data_expr(
            expr,
            &mut |sort: &SortExpression| -> Result<SortExpression, WellTypedError> {
                let flattened = flatten_function_sorts(sort);
                resolve_sort_id(&flattened, &self.sorts)
            },
        )
    }
}

/// Displays a [`DataSpecification`] the same way its own `Display` does, except each equation's
/// own sub-expressions are annotated with their resolved sort (`expr:Sort`) rather than left
/// implicit. Returned by [`DataSpecification::to_typed_string`].
pub struct TypedDataSpecification<'a> {
    spec: &'a DataSpecification,
}

impl fmt::Display for TypedDataSpecification<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let spec = &self.spec.spec;

        if !spec.type_var_declarations.is_empty() {
            writeln!(f, "type_var")?;
            for decl in &spec.type_var_declarations {
                writeln!(f, "   {};", decl.identifier)?;
            }
            writeln!(f)?;
        }
        if !spec.sort_declarations.is_empty() {
            writeln!(f, "sort")?;
            for decl in &spec.sort_declarations {
                writeln!(f, "   {decl};")?;
            }
            writeln!(f)?;
        }
        if !spec.constructor_declarations.is_empty() {
            writeln!(f, "cons")?;
            for decl in &spec.constructor_declarations {
                writeln!(f, "   {decl};")?;
            }
            writeln!(f)?;
        }
        if !spec.map_declarations.is_empty() {
            writeln!(f, "map")?;
            for decl in &spec.map_declarations {
                writeln!(f, "   {decl};")?;
            }
            writeln!(f)?;
        }

        for eqn_spec in &spec.equation_declarations {
            if !eqn_spec.node.variables.is_empty() {
                writeln!(f, "var")?;
                for decl in &eqn_spec.node.variables {
                    writeln!(f, "   {decl};")?;
                }
            }

            writeln!(f, "eqn")?;
            let eqn_spec_id = eqn_spec
                .node
                .id
                .expect("assign_declaration_ids ran during from_untyped");
            for equation in &eqn_spec.node.equations {
                let equation_id = equation.id.expect("assign_declaration_ids ran during from_untyped");
                let typing = self.spec.equation_typing((eqn_spec_id, equation_id));
                let text = TypedRoots::new(Roots::equation(equation), &self.spec.context, &self.spec.spec, typing);
                writeln!(f, "   {text};")?;
            }
        }

        Ok(())
    }
}

/// Returns the target sort of a sort expression, i.e. the range of a function
/// sort, or the sort itself if it is not a function sort.
pub(crate) fn target_sort(sort: &SortExpression) -> &SortExpression {
    debug_assert!(
        !matches!(sort.node, SortExpressionKind::Function { .. }),
        "target_sort should only be called on non-function sorts or flattened function sorts"
    );

    if let SortExpressionKind::FlattenedFunction { domain: _, range } = &sort.node {
        range
    } else {
        sort
    }
}

/// Returns the argument sorts of a (flattened) function sort, or an empty slice
/// for a non-function sort — such as the target sort of a constant constructor
/// like `cons c: S;`, which takes no arguments.
pub(crate) fn argument_sorts(sort: &SortExpression) -> &[SortExpression] {
    if let SortExpressionKind::FlattenedFunction { domain, range: _ } = &sort.node {
        domain
    } else {
        &[]
    }
}

/// Rewrites every `Function` node of `sort` into a `FlattenedFunction` whose
/// domain is the flattened `Product` spine (`(A#B)->C` becomes `A#B->C`).
///
/// `pub(crate)`: also used by `merc_process` (through `crate::checking`) to resolve a binder sort embedded in a
/// process-body expression, the same way it's used below for every sort in the data
/// specification proper (including equation-embedded binder sorts, via `apply_sorts_in_spec`).
pub(crate) fn flatten_function_sorts(sort: &SortExpression) -> SortExpression {
    sort.clone()
        .apply(|expr| -> Result<_, Infallible> {
            if let SortExpressionKind::Function { domain, range } = &expr.node {
                let mut flattened_domain = Vec::new();
                flatten_function_domain_rec(domain, &mut flattened_domain);

                return Ok(Some(
                    SortExpressionKind::FlattenedFunction {
                        domain: flattened_domain,
                        range: range.clone(),
                    }
                    .into(),
                ));
            }

            Ok(None)
        })
        .expect("flatten_function_sorts should not fail")
}

/// Flattens a function sort of the form ((A_0 # A_1) # ... # A_n) -> B into a
/// sort of the form A_0 # A_1 # ... # A_n -> B, where B is the original range
/// of the function.
fn flatten_function_domain_rec(sort: &SortExpression, domain: &mut Vec<SortExpression>) {
    match &sort.node {
        SortExpressionKind::Product { lhs, rhs } => {
            flatten_function_domain_rec(lhs, domain);
            flatten_function_domain_rec(rhs, domain);
        }
        _ => domain.push(sort.clone()),
    }
}

/// A standalone data expression type checked against a [`DataSpecification`], see
/// [`DataSpecification::typecheck_expression`].
pub struct TypedExpression {
    /// The expression, with its variables resolved and its built-in operator nodes rewritten
    /// into applications.
    expr: DataExpr,
    /// The inferred sort of every node of `expr`, and the resolution of every name.
    typing: EquationTyping,
    /// The span-keyed typing of `expr`.
    info: TypingInfo,
}

impl TypedExpression {
    /// The checked expression, with its variables resolved and its built-in operator nodes
    /// (`x + y`, `[x, y]`, `f[x -> y]`) rewritten into applications.
    pub fn expression(&self) -> &DataExpr {
        &self.expr
    }

    /// The typing of the expression, span-keyed so hover/go-to-definition can look up a
    /// sub-expression by source position (see [`TypingInfo::at_offset`]).
    pub fn typing_info(&self) -> &TypingInfo {
        &self.info
    }

    /// The inferred sorts and name resolutions of the expression, indexed by `ExprId`.
    #[cfg_attr(
        not(feature = "internal"),
        expect(dead_code, reason = "only the `internal` API uses it")
    )]
    pub(crate) fn typing(&self) -> &EquationTyping {
        &self.typing
    }
}
