use std::collections::HashSet;
use std::convert::Infallible;
use std::fmt::Write as _;
use std::ops::Range;
use std::sync::Arc;

use log::debug;

use merc_collections::IndexedSet;
use merc_data::DataExpression;
use merc_data::Mcrl2DataSpecification;
use merc_syntax::ConstructorId;
use merc_syntax::DataExpr;
use merc_syntax::EqnSpecId;
use merc_syntax::EquationId;
use merc_syntax::MapId;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::SortId;
use merc_syntax::SourceMap;
use merc_syntax::Traverse;
use merc_syntax::UntypedDataSpecification;
use merc_syntax::VarId;

use crate::AliasError;
use crate::EquationTyping;
use crate::InferenceError;
use crate::NumberEncoding;
use crate::Signature;
use crate::TypeCheckContext;
use crate::TypingInfo;
use crate::VariableSpans;
use crate::WellTypedError;
use crate::apply_sorts_in_data_expr;
use crate::apply_sorts_in_spec;
use crate::assign_declaration_ids;
use crate::basic_sort_data_specification;
use crate::build_signature;
use crate::check_aliases;
use crate::check_comparison_template;
use crate::check_container_templates;
use crate::check_equations;
use crate::check_no_system_function_redeclaration;
use crate::check_products_within_domains;
use crate::check_system_equations;
use crate::check_system_specification;
use crate::desugar_structured_sorts;
use crate::filter_signature;
use crate::hoist_anonymous_structs;
use crate::infer_expression;
use crate::is_basic_sort_name;
use crate::is_well_typed;
use crate::lower_data_expr;
use crate::lower_data_expressions;
use crate::lower_data_specification;
use crate::lower_expression;
use crate::merge_signatures;
use crate::normalize_sorts;
use crate::resolve_data_expr_variables;
use crate::resolve_data_specification_variables;
use crate::resolve_sort;
use crate::resolve_sort_id;
use crate::resolve_sort_ids;
use crate::resolve_system_signature;
use crate::resolve_system_signature_full;
use crate::resolve_type_var_ids;
use crate::resolve_type_vars;
use crate::structured_sort_equations;
use crate::typed_equation_string;
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
            "typecheck: starting on {} sort, {} constructor, {} map and {} equation declaration(s)",
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
            "typecheck: hoisted anonymous structs; {} sort declaration(s) remain",
            spec.sort_declarations.len()
        );

        apply_sorts_in_spec(&mut spec, |sort| -> Result<_, Infallible> {
            Ok(flatten_function_sorts(sort))
        })
        .expect("The inner function never fails");

        resolve_type_vars(&mut spec);

        // Assign ids to `type_var` declarations and resolve every `TypeVar` node to its id.
        resolve_type_var_ids(&mut spec)?;
        debug!("typecheck: resolved type variable name(s)");

        // `basics` depends only on `encoding`, not on `spec`'s own content, so
        // it can be built before name resolution. Only add the non-basic sorts
        // from `basics` to `spec`.
        let mut basics = basic_sort_data_specification(sources, encoding);
        spec.sort_declarations.extend(
            basics
                .sort_declarations
                .iter()
                .filter(|decl| !is_basic_sort_name(&decl.identifier))
                .cloned(),
        );

        // The returned sorts are only used for lookup cycles.
        let sorts = resolve_sort_ids(&mut spec)?;
        debug!("typecheck: resolved {} sort name(s)", sorts.len());

        // `basics`'s own declarations reference `@NatPair`/`@word` by bare name.
        apply_sorts_in_spec(&mut basics, |sort| resolve_sort_id(sort, &sorts))?;

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
            "typecheck: desugared {} structured sort(s) into {} constructor(s)",
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
        debug!("typecheck: signature checks passed");

        // Every sort-name reference `spec`'s own declarations make.
        let sort_references = typing_info::collect_data_specification_sort_references(&spec);
        debug!("typecheck: collected {} sort-name reference(s)", sort_references.len());

        // Expand aliases to a canonical form now that they are known to be
        // acyclic.
        normalize_sorts(&mut spec);
        debug!("typecheck: normalized alias indirection");

        // Safety net over the normalized spec:.
        is_well_typed(&spec)?;
        debug!("typecheck: well-typedness checks passed");

        // Lower the built-in operator nodes in the user equations to named
        // applications.
        lower_data_expressions(&mut spec);
        debug!("typecheck: lowered the user equations");

        // The system-defined part of type-checking is deliberately narrow now:
        // `system` holds only `basics` (the five basic sorts, always present.
        check_no_system_function_redeclaration(&spec, &basics)?;
        debug!("typecheck: no user declaration redeclares a system function");

        let mut system = basics.clone();

        // The defining equations of each structured sort (Appendix B.10) join
        // the system-defined part. Each struct's range and symbol names are
        // recorded so its equations can later be checked against a signature
        // scoped to that struct alone — pooling them would make a name shared
        // with an unrelated struct ambiguous, see `filter_signature`.
        let mut struct_ranges: Vec<(Range<usize>, HashSet<String>, HashSet<String>)> = Vec::new();
        for constructors in &structs {
            let start = system.equation_declarations.len();
            system.merge(&structured_sort_equations(sources, constructors).map_err(WellTypedError::Custom)?);
            let end = system.equation_declarations.len();
            let constructor_names: HashSet<String> = constructors.iter().map(|c| c.name.node.clone()).collect();
            let mapping_names: HashSet<String> = constructors
                .iter()
                .flat_map(|c| {
                    c.projection
                        .clone()
                        .into_iter()
                        .chain(c.args.iter().filter_map(|(name, _)| name.clone()))
                        .map(|spanned| spanned.node)
                })
                .collect();
            struct_ranges.push((start..end, constructor_names, mapping_names));
        }

        // A struct's own equations are generated as fresh source text and
        // re-parsed.
        apply_sorts_in_spec(&mut system, |sort| resolve_sort_id(sort, &sorts))?;

        // The system equations parse with the same operator nodes, so they are
        // lowered like the user equations.
        lower_data_expressions(&mut system);

        debug!(
            "typecheck: built the base system-defined specification with {} sort, {} map and {} equation \
             declaration(s)",
            system.sort_declarations.len(),
            system.map_declarations.len(),
            system.equation_declarations.len()
        );

        // Resolve the system-defined declarations of the *basic* sorts onto
        // the same lattice, so Phase-3 inference sees the overload sets of the
        // built-in operators.
        resolve_system_signature(&mut context, &spec, &basics)?;
        debug!("typecheck: resolved the system signature");

        // Type checks every container/function-update template's own
        // equations once.
        check_container_templates(&mut context, encoding)?;
        // Comparison-operator equations are checked the same way.
        check_comparison_template(&mut context)?;
        debug!("typecheck: container template equations passed the rigid check");

        // Inference over every user equation; an equation binding
        // a variable through an invalid sort (a bare product) is rejected here.
        check_equations(&mut context, &spec, &system)?;
        debug!("typecheck: inference finished; the specification is well-typed");

        // Ties every system equation's own variable occurrences to its `var`-block declaration.
        resolve_data_specification_variables(&mut system);

        // Unconditional in every build (not a debug_assert!): silently trusting
        // a malformed generated spec in release would leave a rewrite spec
        // quietly missing rules.
        check_system_specification(&spec, &system)?;
        debug!(
            "typecheck: final system-defined specification has {} sort, {} map and {} equation declaration(s)",
            system.sort_declarations.len(),
            system.map_declarations.len(),
            system.equation_declarations.len()
        );

        assign_declaration_ids(&mut system);

        resolve_system_signature_full(&mut context, &spec, &system);

        for (range, constructor_names, mapping_names) in &struct_ranges {
            let struct_signature = filter_signature(
                context.signature.as_deref().expect("build_signature ran earlier"),
                constructor_names,
                mapping_names,
            );
            let signature = Arc::new(merge_signatures(
                &struct_signature,
                context
                    .basics_signature
                    .as_deref()
                    .expect("resolve_system_signature ran earlier"),
            ));
            for i in range.clone() {
                context
                    .struct_signature_overrides
                    .insert(EqnSpecId::new(i), Arc::clone(&signature));
            }
        }
        debug!("typecheck: resolved the system-equation signatures");

        // `system` at this point holds only `basics` and the desugared
        // structs' own equations).
        check_system_equations(&mut context, &spec, &system, &[])?;
        debug!("typecheck: system-equation inference finished; the system specification is well-typed");

        Ok(Self {
            spec,
            sorts,
            system,
            context,
            encoding,
            sort_references,
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

    /// The query context holding the sorts interned during type checking,
    /// backing the declaration-sort accessors below.
    // Currently exercised by tests only.
    #[allow(dead_code)]
    pub(crate) fn context(&self) -> &TypeCheckContext {
        &self.context
    }

    /// The resolved sort of the constructor declaration with the given
    /// [ConstructorId]. Requires `id` to be a valid constructor id from this
    /// specification; panics if called before `from_untyped` has completed.
    ///
    /// Only ever safe for a *user* declaration's id: every one of those has its sort resolved
    /// unconditionally during `from_untyped`, whether or not anything actually references it. A
    /// system-defined declaration's id is not resolved this way at all — see
    /// [`TypeCheckContext::system_symbol_spans`]'s doc comment for where that comes from instead.
    pub(crate) fn sort_of_constructor(&self, id: ConstructorId) -> crate::ResolvedSortId {
        self.context
            .sort_of_constructor
            .get(&id)
            .copied()
            .expect("constructor sorts are all resolved during from_untyped")
    }

    /// The resolved sort of the map declaration with the given [MapId].
    /// Requires `id` to be a valid map id from this specification; panics if
    /// called before `from_untyped` has completed.
    ///
    /// See [`Self::sort_of_constructor`]'s doc comment: safe only for a *user* declaration's id.
    pub(crate) fn sort_of_map(&self, id: MapId) -> crate::ResolvedSortId {
        self.context
            .sort_of_map
            .get(&id)
            .copied()
            .expect("map sorts are all resolved during from_untyped")
    }

    /// The resolved sort of the equation `var`-block variable identified by `var_id`. Requires
    /// `var_id` to be valid from this specification; panics if called before `from_untyped` has
    /// completed.
    // Currently exercised by tests only.
    #[allow(dead_code)]
    pub(crate) fn sort_of_equation_var(&self, var_id: VarId) -> crate::ResolvedSortId {
        self.context
            .sort_of_equation_var
            .get(&var_id)
            .copied()
            .expect("equation variable sorts are all resolved during from_untyped")
    }

    /// The (S, C, M) signature: the resolved overload sets of every constructor
    /// and mapping name.
    // Currently exercised by tests only.
    #[allow(dead_code)]
    pub(crate) fn signature(&self) -> &Signature {
        self.context
            .signature
            .as_deref()
            .expect("build_signature ran in from_untyped")
    }

    /// The Phase-3 typing of the equation identified by `key`, read from the
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

    /// Assembles and returns the fully typed mCRL2 data specification in the
    /// binary aterm format.
    ///
    /// Includes the user sort declarations, aliases, constructors, mappings,
    /// and equations. Call this once after [`Self::from_untyped`] when the
    /// lowered typed specification is needed.
    ///
    /// `self.system` is already extended and checked by `from_untyped_with`, so
    /// this is a pure read-only replay.
    pub fn lower_data_specification(&self) -> Mcrl2DataSpecification {
        lower_data_specification(&self.context, &self.spec, &self.system, self.encoding)
    }

    /// Renders `self.data_specification()` the same way its own `Display` does
    /// except each equation's own sub-expressions are annotated with their
    /// resolved sort (`expr:Sort`) rather than left implicit.
    pub fn to_typed_string(&self) -> String {
        let spec = &self.spec;
        let mut out = String::new();

        if !spec.type_var_declarations.is_empty() {
            out.push_str("type_var\n");
            for decl in &spec.type_var_declarations {
                let _ = writeln!(out, "   {};", decl.identifier);
            }
            out.push('\n');
        }
        if !spec.sort_declarations.is_empty() {
            out.push_str("sort\n");
            for decl in &spec.sort_declarations {
                let _ = writeln!(out, "   {decl};");
            }
            out.push('\n');
        }
        if !spec.constructor_declarations.is_empty() {
            out.push_str("cons\n");
            for decl in &spec.constructor_declarations {
                let _ = writeln!(out, "   {decl};");
            }
            out.push('\n');
        }
        if !spec.map_declarations.is_empty() {
            out.push_str("map\n");
            for decl in &spec.map_declarations {
                let _ = writeln!(out, "   {decl};");
            }
            out.push('\n');
        }

        for eqn_spec in &spec.equation_declarations {
            if !eqn_spec.node.variables.is_empty() {
                out.push_str("var\n");
                for decl in &eqn_spec.node.variables {
                    let _ = writeln!(out, "   {decl};");
                }
            }

            out.push_str("eqn\n");
            let eqn_spec_id = eqn_spec
                .node
                .id
                .expect("assign_declaration_ids ran during from_untyped");
            for equation in &eqn_spec.node.equations {
                let equation_id = equation.id.expect("assign_declaration_ids ran during from_untyped");
                let typing = self.equation_typing((eqn_spec_id, equation_id));
                let text = typed_equation_string(equation, &self.context, &self.spec, typing);
                let _ = writeln!(out, "   {text};");
            }
        }

        out
    }

    /// Type checks a single data expression against this specification and
    /// lowers it to the same aterm form [`Self::lower_data_specification`]
    /// produces, so the result can be handed straight to a rewriter built from
    /// that specification.
    ///
    /// `expr` is a *closed* term: it may use any constructor or mapping this
    /// specification declares (user or system-defined) and may introduce its own
    /// bound variables through `lambda`/`forall`/`exists`/a comprehension/`whr`,
    /// but a free identifier is an [`InferenceError::UndeclaredName`] — there is
    /// no enclosing `var` block to draw equation variables from. Sorts are
    /// inferred exactly as in a user equation, except that no other side widens
    /// the result: `1 + 1` types at `Pos`, its minimal sort.
    ///
    /// Takes `&mut self` because inference interns the sorts it discovers into
    /// the shared context; the specification itself is not modified.
    ///
    /// # Panics
    ///
    /// Panics if the expression type checks but Phase-4 lowering cannot render
    /// it — an internal inconsistency between the two phases, treated the same
    /// way as for a user equation in [`Self::lower_data_specification`].
    pub fn typecheck_expression(&mut self, expr: &DataExpr) -> Result<DataExpression, InferenceError> {
        self.typecheck_expression_with_typing(expr)
            .map(|(lowered, _typing_info)| lowered)
    }

    /// As [`Self::typecheck_expression`], additionally returning `expr`'s [`TypingInfo`] — the
    /// same information [`Self::equation_typing_info`] exposes for a user equation, span-keyed so
    /// a caller can look up the sort or name resolution of any sub-expression by source position
    /// (see [`TypingInfo::at_offset`]). `expr` here is the caller's own, unlowered expression, so
    /// `TypingInfo`'s spans line up with the text the caller parsed it from.
    ///
    /// # Panics
    ///
    /// Same as [`Self::typecheck_expression`].
    pub fn typecheck_expression_with_typing(
        &mut self,
        expr: &DataExpr,
    ) -> Result<(DataExpression, TypingInfo), InferenceError> {
        // Ties every local binder this.
        let mut expr = expr.clone();
        resolve_data_expr_variables(&mut expr);

        // `expr`'s own binders (a `lambda`/`forall`/`exists`/comprehension/`whr`) each already
        // carry their own `VarId` and declaring span after resolution above; collected here, from
        // `expr` itself, before `lower_data_expr` below consumes it.
        let mut variable_spans = VariableSpans::new();
        typing_info::collect_data_expr_variable_declarations(&expr, &mut variable_spans);

        // The built-in operator nodes (`x + y`, `[x, y]`, `f[x -> y]`) become
        // applications first, exactly as `from_untyped_with` does for the
        // equations: inference and lowering both require a lowered expression.
        let lowered_expr = lower_data_expr(expr);

        let typing = infer_expression(&mut self.context, &self.spec, &lowered_expr)?;
        let info = typing_info::build(self, &typing, &variable_spans);

        let lowered = lower_expression(&self.context, &self.spec, &typing, &lowered_expr, self.encoding)
            .unwrap_or_else(|| panic!("expression '{lowered_expr}' passed inference but failed lowering"));
        Ok((lowered, info))
    }

    /// The typing of one user equation, span-keyed so hover/go-to-definition can look up a
    /// sub-expression by source position (see [`TypingInfo::at_offset`]).
    ///
    /// `key` must index an equation of this specification (the `EqnSpecId`/`EquationId` on
    /// [`Self::data_specification`]); panics otherwise.
    ///
    /// Memoized in `self.context`'s `equation_typing_info` cache: `self` is immutable once built,
    /// so a given `key`'s `TypingInfo` is only ever built once and every later call reuses the
    /// cached `Arc`. Takes `&mut self` to populate that cache, and returns an owned `TypingInfo`
    /// cloned out of it.
    pub fn equation_typing_info(&mut self, key: (EqnSpecId, EquationId)) -> TypingInfo {
        if let Some(cached) = self.context.equation_typing_info.get(&key) {
            return (**cached).clone();
        }
        let (eqn_spec_id, _) = key;
        let variable_spans =
            typing_info::collect_equation_variable_declarations(&self.spec.equation_declarations[*eqn_spec_id]);
        let info = Arc::new(typing_info::build(self, self.equation_typing(key), &variable_spans));
        self.context.equation_typing_info.insert(key, Arc::clone(&info));
        (*info).clone()
    }

    /// Every user equation's typing, merged into one table, in declaration order, plus a
    /// [`crate::ResolvedName::Sort`] node for every sort-name reference this specification's own
    /// declarations make (`cons`/`map` signatures, a `var`-block, a sort alias's own right-hand
    /// side, a binder inside an equation) — see [`Self::equation_typing_info`] for the per-
    /// equation version, which does *not* include these: a sort declaration isn't scoped to any
    /// one equation, so there is nothing meaningful to slice per `key` the way the rest of this
    /// method's result is.
    ///
    /// Memoized as `self.context`'s `whole_typing_info` singleton: a second call reuses the
    /// cached `Arc` instead of re-merging every equation's typing.
    pub fn typing_info(&mut self) -> TypingInfo {
        if let Some(cached) = &self.context.whole_typing_info {
            return (**cached).clone();
        }

        // Collected up front so the loop below can call `self.equation_typing_info` (`&mut self`)
        // without also holding a borrow of `self.spec.equation_declarations`.
        let keys: Vec<(EqnSpecId, EquationId)> = self
            .spec
            .equation_declarations
            .iter()
            .flat_map(|eqn_spec| {
                let eqn_spec_id = eqn_spec.id.expect("assign_declaration_ids ran during from_untyped");
                eqn_spec.equations.iter().map(move |equation| {
                    let equation_id = equation.id.expect("assign_declaration_ids ran during from_untyped");
                    (eqn_spec_id, equation_id)
                })
            })
            .collect();

        let mut info = TypingInfo::default();
        for key in keys {
            info.merge(self.equation_typing_info(key));
        }
        typing_info::push_sort_references(self, &self.sort_references, &mut info);
        self.context.whole_typing_info = Some(Arc::new(info.clone()));
        info
    }

    /// Resolves a sort expression that occurs *outside* the data specification proper — an
    /// action argument, a process parameter, a global variable declaration — against this
    /// already-resolved specification's declared sort names.
    ///
    /// Requires `sort` to contain no anonymous `struct` (see [`crate::process`]).
    pub(crate) fn resolve_declared_sort(
        &mut self,
        sort: &SortExpression,
    ) -> Result<crate::ResolvedSortId, WellTypedError> {
        check_products_within_domains(sort)?;
        let resolved = resolve_sort_id(sort, &self.sorts)?;
        Ok(resolve_sort(&mut self.context, &self.spec, &resolved))
    }

    /// Splits into simultaneous borrows of the query context and the resolved specification, for
    /// a caller (process-level checking, see [`crate::process`]) that needs to run inference
    /// against a variable scope of its own rather than one of `self`'s own equations.
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
/// `pub(crate)`: also used by [`crate::process`] to resolve a binder sort embedded in a
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use merc_syntax::EqnSpecId;
    use merc_syntax::EquationId;
    use merc_syntax::UntypedDataSpecification;

    use crate::DataSpecification;
    use crate::query_equation_typing;

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_equation_typing_is_memoized() {
        let spec = UntypedDataSpecification::parse("map f: Nat; eqn f = 1;").unwrap();
        let mut checked = DataSpecification::from_untyped(spec).unwrap();

        let key = (EqnSpecId::new(0), EquationId::new(0));

        // `from_untyped` already inferred and cached this equation; querying
        // again must return the very same `Arc`, not recompute.
        let first = Arc::clone(
            checked
                .context
                .equation_typing
                .get(&key)
                .expect("from_untyped inferred the equation")
                .as_ref()
                .expect("the equation is well-typed"),
        );
        let again = query_equation_typing(&mut checked.context, &checked.spec, &checked.system, key).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
    }

    /// A second `equation_typing_info` call for the same key must reuse the cached `Arc` rather
    /// than re-deriving the `DeclarationIndex` and every node's sort again.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_equation_typing_info_is_memoized() {
        let spec = UntypedDataSpecification::parse("map f: Nat; eqn f = 1;").unwrap();
        let mut checked = DataSpecification::from_untyped(spec).unwrap();
        let key = (EqnSpecId::new(0), EquationId::new(0));

        checked.equation_typing_info(key);
        let first = Arc::clone(
            checked
                .context
                .equation_typing_info
                .get(&key)
                .expect("equation_typing_info populated the cache"),
        );
        checked.equation_typing_info(key);
        let again = Arc::clone(checked.context.equation_typing_info.get(&key).expect("still cached"));
        assert!(
            Arc::ptr_eq(&first, &again),
            "a second call must reuse the cached TypingInfo, not rebuild it"
        );
    }

    /// As [`test_equation_typing_info_is_memoized`], for the whole-document `typing_info` memo.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_typing_info_is_memoized() {
        let spec = UntypedDataSpecification::parse("map f: Nat; eqn f = 1;").unwrap();
        let mut checked = DataSpecification::from_untyped(spec).unwrap();

        checked.typing_info();
        let first = Arc::clone(
            checked
                .context
                .whole_typing_info
                .as_ref()
                .expect("typing_info populated the cache"),
        );
        checked.typing_info();
        let again = Arc::clone(checked.context.whole_typing_info.as_ref().expect("still cached"));
        assert!(
            Arc::ptr_eq(&first, &again),
            "a second call must reuse the cached TypingInfo, not rebuild it"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_mcrl2_data_specification_sections_populated() {
        let spec = DataSpecification::from_untyped(
            UntypedDataSpecification::parse(
                "sort D; \
                 sort A = Nat; \
                 cons c: D; \
                 map f: D -> Bool; \
                 var d: D; \
                 eqn f(d) = true;",
            )
            .unwrap(),
        )
        .unwrap();
        let mcrl2 = spec.lower_data_specification();

        // User abstract sort `D` → sorts section.
        assert!(mcrl2.sorts().iter().any(|s| s.name() == "D"), "D must appear in sorts");
        // User alias `A = Nat` → aliases section.
        assert!(
            mcrl2.aliases().iter().any(|a| a.name().name() == "A"),
            "A must appear in aliases"
        );
        // User constructor `c` → constructors section.
        assert!(
            mcrl2.constructors().iter().any(|c| c.name() == "c"),
            "c must appear in constructors"
        );
        // User mapping `f` → mappings section.
        assert!(
            mcrl2.mappings().iter().any(|m| m.name() == "f"),
            "f must appear in mappings"
        );
        // User equation `f(d) = true` → equations section.
        assert!(
            !mcrl2.equations().is_empty(),
            "at least one user equation must be lowered"
        );
        assert_eq!(mcrl2.equations()[0].lhs().to_string(), "f(d)");
        assert_eq!(mcrl2.equations()[0].rhs().to_string(), "true");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_mcrl2_data_specification_system_constructors_present() {
        // `Bool` always pulls in its system constructors; at least `true`/`false` must appear.
        let spec = DataSpecification::from_untyped(UntypedDataSpecification::parse("map f: Bool;").unwrap()).unwrap();
        let mcrl2 = spec.lower_data_specification();
        assert!(
            mcrl2.constructors().iter().any(|c| c.name() == "true"),
            "system Bool constructor `true` must appear in constructors"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_mcrl2_data_specification_system_equations_present() {
        // System Bool equations (e.g. `!true = false`) must appear now that
        // `lower_data_specification` includes structurally-lowerable system equations.
        let spec = DataSpecification::from_untyped(UntypedDataSpecification::parse("map f: Bool;").unwrap()).unwrap();
        let mcrl2 = spec.lower_data_specification();
        // `!true = false` should be among the system Bool equations.
        let found = mcrl2
            .equations()
            .iter()
            .any(|e| e.lhs().to_string().contains("!(true)") && e.rhs().to_string() == "false");
        assert!(found, "system Bool equation `!(true) = false` must be present");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_mcrl2_data_specification_empty_container_equations_present() {
        // A `List(D)` sort pulls in the Appendix-B list equations, several of
        // which mention the empty-list literal `[]` (`in(d, []) = false`,
        // `#[] = @c0`). These are now lowered structurally via expected-sort
        // propagation rather than skipped.
        let spec = DataSpecification::from_untyped(
            UntypedDataSpecification::parse("sort D; map f: List(D) -> Bool;").unwrap(),
        )
        .unwrap();
        let mcrl2 = spec.lower_data_specification();

        // `in(d, []) = false` — the empty list as a `List(D)` argument.
        let in_empty = mcrl2
            .equations()
            .iter()
            .any(|e| e.lhs().to_string() == "in(d, [])" && e.rhs().to_string() == "false");
        assert!(in_empty, "system list equation `in(d, []) = false` must be present");

        // `#[] = @c0` — the empty list under the length operator.
        let length_empty = mcrl2
            .equations()
            .iter()
            .any(|e| e.lhs().to_string() == "#([])" && e.rhs().to_string() == "@c0");
        assert!(length_empty, "system list equation `#[] = @c0` must be present");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_mcrl2_data_specification_enumeration_literal_container_equations_present() {
        // `List(Nat)` never occurs as a textual sort here — only as the
        // element sort of the list-enumeration literal `[1, 2, 3]`, which
        // `collect_system_sorts_in_spec`'s syntactic scan cannot see (its own
        // doc comment notes enumeration literals are not syntactically
        // apparent). Its Appendix-B equations must still be instantiated from
        // the inferred sort during lowering.
        let spec = DataSpecification::from_untyped(
            UntypedDataSpecification::parse("map f: Bool; eqn f = 1 in [2, 3];").unwrap(),
        )
        .unwrap();
        let mcrl2 = spec.lower_data_specification();

        let in_empty = mcrl2
            .equations()
            .iter()
            .any(|e| e.lhs().to_string() == "in(d, [])" && e.rhs().to_string() == "false");
        assert!(
            in_empty,
            "system list equation `in(d, []) = false` for List(Nat) must be present: {:#?}",
            mcrl2.equations().iter().map(|e| e.to_string()).collect::<Vec<_>>()
        );

        // The `|>` (cons) constructor for `List(Nat)` must also be declared,
        // not just its equations.
        assert!(
            mcrl2.constructors().iter().any(|c| c.name() == "|>"),
            "the List(Nat) cons constructor must be present"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_mcrl2_data_specification_set_enumeration_literal_container_equations_present() {
        // As above, but for a set-enumeration literal (`FSet(Nat)`, never
        // declared textually).
        let spec = DataSpecification::from_untyped(
            UntypedDataSpecification::parse("map n: Nat; eqn n = #{1, 2, 3};").unwrap(),
        )
        .unwrap();
        let mcrl2 = spec.lower_data_specification();

        let in_empty = mcrl2
            .equations()
            .iter()
            .any(|e| e.lhs().to_string() == "in(d, {})" && e.rhs().to_string() == "false");
        assert!(
            in_empty,
            "system fset equation `in(d, {{}}) = false` for FSet(Nat) must be present: {:#?}",
            mcrl2.equations().iter().map(|e| e.to_string()).collect::<Vec<_>>()
        );
        assert!(
            mcrl2.constructors().iter().any(|c| c.name() == "@fset_insert"),
            "the FSet(Nat) @fset_insert constructor must be present"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_mcrl2_data_specification_bag_enumeration_literal_container_equations_present() {
        // As above, but for a bag-enumeration literal (`FBag(Nat)`, never
        // declared textually).
        let spec = DataSpecification::from_untyped(
            UntypedDataSpecification::parse("map n: Nat; eqn n = #{1: 2, 3: 4};").unwrap(),
        )
        .unwrap();
        let mcrl2 = spec.lower_data_specification();

        assert!(
            mcrl2.mappings().iter().any(|m| m.name() == "@fbag_cinsert"),
            "the FBag(Nat) @fbag_cinsert mapping must be present: {:#?}",
            mcrl2
                .mappings()
                .iter()
                .map(|m| m.name().to_string())
                .collect::<Vec<_>>()
        );
    }

    /// Formats every lowered equation as `lhs = rhs`, for assertion messages.
    fn equation_strings(mcrl2: &merc_data::Mcrl2DataSpecification) -> Vec<String> {
        mcrl2
            .equations()
            .iter()
            .map(|e| format!("{} = {}", e.lhs(), e.rhs()))
            .collect()
    }

    // The tests below guard that a system equation using a binder, a bare
    // higher-order name value, or a struct-desugared symbol reaches the lowered
    // output rather than being dropped.

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_set_extensionality_equation_survives_lowering() {
        // `set.mcrl2`'s `@set(f, s) == @set(g, t) = forall c:S. ...`.
        let spec =
            DataSpecification::from_untyped(UntypedDataSpecification::parse("map f: Set(Nat) -> Bool;").unwrap())
                .unwrap();
        let mcrl2 = spec.lower_data_specification();
        let found = mcrl2
            .equations()
            .iter()
            .any(|e| e.lhs().to_string().contains("==") && e.rhs().to_string().contains("Forall"));
        assert!(
            found,
            "the Set extensionality equation (a forall in its rhs) must survive lowering: {:#?}",
            equation_strings(&mcrl2)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_bag_extensionality_equation_survives_lowering() {
        // `bag.mcrl2`'s counterpart of the Set extensionality equation.
        let spec =
            DataSpecification::from_untyped(UntypedDataSpecification::parse("map f: Bag(Nat) -> Bool;").unwrap())
                .unwrap();
        let mcrl2 = spec.lower_data_specification();
        let found = mcrl2
            .equations()
            .iter()
            .any(|e| e.lhs().to_string().contains("==") && e.rhs().to_string().contains("Forall"));
        assert!(
            found,
            "the Bag extensionality equation (a forall in its rhs) must survive lowering: {:#?}",
            equation_strings(&mcrl2)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_bare_higher_order_value_equation_survives_lowering() {
        // `set.mcrl2`'s `@setfset(s) = @set(@false_, s)`, where `@false_` is used
        // point-free (`S -> Bool`, never applied).
        let spec =
            DataSpecification::from_untyped(UntypedDataSpecification::parse("map f: Set(Nat) -> Bool;").unwrap())
                .unwrap();
        let mcrl2 = spec.lower_data_specification();
        let found = mcrl2
            .equations()
            .iter()
            .any(|e| e.lhs().to_string() == "@setfset(s)" && e.rhs().to_string() == "@set(@false_, s)");
        assert!(
            found,
            "the '@setfset(s) = @set(@false_, s)' equation must survive lowering: {:#?}",
            equation_strings(&mcrl2)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_struct_recogniser_and_projection_equations_survive_lowering() {
        // A struct's recogniser/projection equations reference symbols (`is_c1`,
        // `pr1`, `c1`) declared on the user spec by struct desugaring, not on
        // the system spec.
        let spec = DataSpecification::from_untyped(
            UntypedDataSpecification::parse(
                "sort D = struct c1(pr1: Nat, pr2: Bool)?is_c1 | c2?is_c2; map f: D -> Bool;",
            )
            .unwrap(),
        )
        .unwrap();
        let mcrl2 = spec.lower_data_specification();

        let recogniser = mcrl2
            .equations()
            .iter()
            .any(|e| e.lhs().to_string() == "is_c1(c1(x0_0, x0_1))" && e.rhs().to_string() == "true");
        assert!(
            recogniser,
            "the recogniser equation 'is_c1(c1(x0_0, x0_1)) = true' must survive lowering: {:#?}",
            equation_strings(&mcrl2)
        );

        let projection = mcrl2
            .equations()
            .iter()
            .any(|e| e.lhs().to_string() == "pr1(c1(x0_0, x0_1))" && e.rhs().to_string() == "x0_0");
        assert!(
            projection,
            "the projection equation 'pr1(c1(x0_0, x0_1)) = x0_0' must survive lowering: {:#?}",
            equation_strings(&mcrl2)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_multiple_element_sorts_of_the_same_container_do_not_collide() {
        // `Bag(Nat)` and `Bag(D)` each carry their own copy of `bag.mcrl2`'s
        // `@zero_ == @one_ = false;`, which pins down no instantiation and would
        // be ambiguous against one pooled signature — see `SystemEquationGroup`.
        let spec = DataSpecification::from_untyped(
            UntypedDataSpecification::parse("sort D = struct d1; map f: Bag(Nat) -> Bool; g: Bag(D) -> Bool;").unwrap(),
        )
        .unwrap();
        let mcrl2 = spec.lower_data_specification();

        assert_eq!(
            mcrl2.mappings().iter().filter(|m| m.name() == "@zero_").count(),
            2,
            "both Bag(Nat) and Bag(D) should declare their own @zero_: {:#?}",
            mcrl2
                .mappings()
                .iter()
                .map(|m| m.name().to_string())
                .collect::<Vec<_>>()
        );

        let zero_eq_one_false_count = mcrl2
            .equations()
            .iter()
            .filter(|e| e.lhs().to_string() == "==(@zero_, @one_)" && e.rhs().to_string() == "false")
            .count();
        assert_eq!(
            zero_eq_one_false_count,
            2,
            "'@zero_ == @one_ = false' should survive once per instantiation: {:#?}",
            equation_strings(&mcrl2)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_struct_constant_and_unrelated_projection_sharing_a_name_do_not_collide() {
        // `a` is both struct A's nullary constant and an unrelated struct's
        // projection — a constructor-vs-mapping overload of one name, which
        // would make A's own `a == a = true` ambiguous if the two were pooled.
        let spec = DataSpecification::from_untyped(
            UntypedDataSpecification::parse(
                "sort A = struct a?is_a; \
                 sort APos = struct ca(a: A)?is_ca | cpos(p: Pos)?is_cpos; \
                 map f: A -> Bool;",
            )
            .unwrap(),
        )
        .unwrap();
        let mcrl2 = spec.lower_data_specification();

        let reflexivity = mcrl2
            .equations()
            .iter()
            .any(|e| e.lhs().to_string() == "==(a, a)" && e.rhs().to_string() == "true");
        assert!(
            reflexivity,
            "struct A's own 'a == a = true' equation must survive, unambiguously: {:#?}",
            equation_strings(&mcrl2)
        );
    }

    /// Tests that `to_typed_string` correctly annotates every sub-expression
    /// with its resolved sort.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_to_typed_string_annotates_every_subexpression() {
        let spec = DataSpecification::from_untyped(
            UntypedDataSpecification::parse(
                "sort Signal; Message;
                 map AssocReq: Nat -> Message;
                     signal: Signal -> Message;
                     sig_AssocReq: Nat -> Signal;
                 var t: Nat;
                 eqn AssocReq(t) = signal(sig_AssocReq(t));",
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            spec.to_typed_string(),
            // `@NatPair` is the one system-internal nominal sort folded into the
            // shared `sort_declarations` table alongside the user's own (see
            // `docs/typecheck.md`'s `DefId`-offset milestone) — present here
            // regardless of whether this spec ever uses it.
            "sort\n\
             \u{20}  Signal;\n\
             \u{20}  Message;\n\
             \u{20}  @NatPair;\n\
             \n\
             map\n\
             \u{20}  AssocReq: (Nat -> Message);\n\
             \u{20}  signal: (Signal -> Message);\n\
             \u{20}  sig_AssocReq: (Nat -> Signal);\n\
             \n\
             var\n\
             \u{20}  t: Nat;\n\
             eqn\n\
             \u{20}  AssocReq(t: Nat): (Nat -> Message) = \
             signal(sig_AssocReq(t: Nat): (Nat -> Signal)): (Signal -> Message);\n"
        );
    }

    /// As above, over the polymorphic comparison/`if` schemes and an implicit `Pos -> Nat`
    /// upcast: every operator's own resolved overload is visible, not just the equation's
    /// declared result.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_to_typed_string_shows_the_resolved_overload_of_a_polymorphic_operator() {
        let spec = DataSpecification::from_untyped(
            UntypedDataSpecification::parse(
                "map f: Nat -> Bool;
                 var i: Nat;
                 eqn f(i) = if(i == 1, true, false);",
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            spec.to_typed_string(),
            // `@NatPair`, folded into `sort_declarations` unconditionally — see
            // the other `to_typed_string` test's comment.
            "sort\n\
             \u{20}  @NatPair;\n\
             \n\
             map\n\
             \u{20}  f: (Nat -> Bool);\n\
             \n\
             var\n\
             \u{20}  i: Nat;\n\
             eqn\n\
             \u{20}  f(i: Nat): (Nat -> Bool) = \
             if(==(i: Nat, 1: Pos): (Nat # Nat -> Bool), true: Bool, false: Bool): \
             (Bool # Bool # Bool -> Bool);\n"
        );
    }
}
