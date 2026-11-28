use std::cmp::Ordering;
use std::collections::HashMap;

use crate::system_symbols;
use merc_data::BasicSort;
use merc_data::BinderType;
use merc_data::ContainerSortKind;
use merc_data::DataAbstraction;
use merc_data::DataApplication;
use merc_data::DataEquation;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::DataWhereClause;
use merc_data::DataWhrDecl;
use merc_data::MachineNumber;
use merc_data::Mcrl2DataSpecification;
use merc_data::SortAlias;
use merc_data::SortArrow;
use merc_data::SortCons;
use merc_data::SortExpression as DataSortExpression;
use merc_syntax::BagElement;
use merc_syntax::ComplexSort;
use merc_syntax::DataExpr;
use merc_syntax::DataExprKind;
use merc_syntax::Quantifier;
use merc_syntax::Sort;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::UntypedDataSpecification;

use merc_typecheck::DataSpecification;
use merc_typecheck::NumberEncoding;
use merc_typecheck::TypedExpression;
use merc_typecheck::internal::EquationTyping;
use merc_typecheck::internal::ExprId;
use merc_typecheck::internal::NameTarget;
use merc_typecheck::internal::ResolvedSort;
use merc_typecheck::internal::ResolvedSortId;
use merc_typecheck::internal::TypeCheckContext;
use merc_typecheck::internal::context;
use merc_typecheck::internal::expression_typing;
use merc_typecheck::internal::instantiate_used_templates;
use merc_typecheck::internal::is_system_generated_name;
use merc_typecheck::internal::unreachable_not_a_value_sort;

/// The merc_data container kind for a [ComplexSort]; the two enums are kept
/// separate because `merc_data` sits below `merc_syntax` in the dependency
/// layering and cannot name it directly.
fn container_kind(op: ComplexSort) -> ContainerSortKind {
    match op {
        ComplexSort::List => ContainerSortKind::List,
        ComplexSort::Set => ContainerSortKind::Set,
        ComplexSort::FSet => ContainerSortKind::FSet,
        ComplexSort::FBag => ContainerSortKind::FBag,
        ComplexSort::Bag => ContainerSortKind::Bag,
    }
}

/// Widens `term` one step up the number lattice (`Pos <= Nat <= Int <= Real`),
/// returning the wrapped term and its new sort. The coercion is the explicit
/// constructor chain, not a named `Pos2Nat`/… conversion function — those are
/// rewrite rules that reduce to exactly these constructor applications
/// (`nat.mcrl2`/`int.mcrl2`/`real.mcrl2`). Steps compose for a non-adjacent
/// pair (e.g. `Pos -> Real` becomes `@cReal(@cInt(@cNat(x)), @c1)`, not a
/// single `Pos2Real` call).
fn widen_one_step(term: DataExpression, from: Sort, encoding: NumberEncoding) -> (DataExpression, Sort) {
    match from {
        Sort::Pos => {
            // The binary encoding embeds a `Pos` into `Nat` with the `@cNat`
            // constructor. The machine-word encoding has no such constructor —
            // a `Nat` digit chain is headed by `@most_significant_digitNat` —
            // so it uses the `Pos2Nat` mapping, whose equations in
            // `nat64.mcrl2` rewrite it onto the corresponding digit chain.
            let convert = match encoding {
                NumberEncoding::Binary => system_symbols::c_nat(),
                NumberEncoding::MachineWord => system_symbols::pos2nat(),
            };
            (DataApplication::with_args(&convert, &[term]).into(), Sort::Nat)
        }
        Sort::Nat => (
            DataApplication::with_args(&system_symbols::c_int(), &[term]).into(),
            Sort::Int,
        ),
        Sort::Int => (
            DataApplication::with_args(&system_symbols::c_real(), &[term, pos_literal("1", encoding)]).into(),
            Sort::Real,
        ),
        Sort::Real | Sort::Bool => unreachable!("Real/Bool never widen further"),
    }
}

/// Widens `term` from `from` to `to` in the number lattice, composing
/// [widen_one_step] as many times as needed.
fn numeric_coerce(mut term: DataExpression, from: Sort, to: Sort, encoding: NumberEncoding) -> DataExpression {
    let mut current = from;
    while current != to {
        (term, current) = widen_one_step(term, current, encoding);
    }
    term
}

/// Widens `term`, an `FSet(element)`/`FBag(element)`, to `Set(element)`/
/// `Bag(element)` via the constructor `@set(@false_, term)` /
/// `@bag(@zero_, term)` — not a call to `@setfset`/`@bagfbag`, which are
/// rewrite-system-only operators (`set.mcrl2` itself notes `@setfset`
/// "should not be part of the rewrite system").
fn container_coerce(term: DataExpression, op: ComplexSort, element: DataSortExpression) -> DataExpression {
    match op {
        ComplexSort::FSet => DataApplication::with_args(
            &system_symbols::set_constructor(&element),
            &[system_symbols::false_function(&element).into(), term],
        )
        .into(),
        ComplexSort::FBag => DataApplication::with_args(
            &system_symbols::bag_constructor(&element),
            &[system_symbols::zero_function(&element).into(), term],
        )
        .into(),
        ComplexSort::List | ComplexSort::Set | ComplexSort::Bag => {
            unreachable!("only FSet and FBag widen to another container")
        }
    }
}

/// Converts an inferred, interned sort into the aterm `SortExpression` the
/// binary format uses: `Primitive`/`Generic`/`Function` recurse structurally
/// onto `BasicSort`/`SortCons`/`SortArrow`, and `Def` resolves to its declared
/// name via [TypeCheckContext::sort_display_name] — a user sort or a
/// system-internal one alike, both declared in `spec`, or a synthesized
/// placeholder as a last resort: a nominal sort's identity *is* its declared
/// name for the binary schema.
pub(crate) fn lower_sort(
    ctx: &TypeCheckContext,
    spec: &UntypedDataSpecification,
    id: ResolvedSortId,
) -> DataSortExpression {
    match ctx.sorts.get(id) {
        ResolvedSort::Primitive(sort) => BasicSort::new(sort.name()).into(),
        ResolvedSort::Container { op, subsort } => {
            SortCons::new(container_kind(*op), lower_sort(ctx, spec, *subsort)).into()
        }
        ResolvedSort::Function { domain, range } => {
            let domain: Vec<DataSortExpression> = domain.iter().map(|&sort| lower_sort(ctx, spec, sort)).collect();
            SortArrow::new(&domain, &lower_sort(ctx, spec, *range)).into()
        }
        ResolvedSort::Def(def) => BasicSort::new(ctx.sort_display_name(spec, *def).as_ref()).into(),
        ResolvedSort::TypeVar(_) => unreachable_not_a_value_sort("Var"),
    }
}

/// Number of bits in one machine-word digit; the digit base is `2^WORD_BITS`.
const WORD_BITS: usize = 64;

/// Builds the machine-number term for a single `@word` digit.
fn machine_number(value: u64) -> DataExpression {
    MachineNumber::new(value).into()
}

/// The binary digits of a non-negative decimal literal, least-significant
/// first, computed by repeated long division by two on the decimal digits
/// (so arbitrarily large literals need no fixed-width integer type). The
/// last element is always `true`: a positive number's leading bit is set by
/// definition, and `"0"` is never passed in (see [pos_literal]).
fn decimal_bits_lsb_first(decimal: &str) -> Vec<bool> {
    let mut digits: Vec<u8> = decimal.bytes().map(|b| b - b'0').collect();
    let mut bits = Vec::new();
    while !(digits.len() == 1 && digits[0] == 0) {
        let mut remainder = 0u8;
        for digit in &mut digits {
            let value = remainder * 10 + *digit;
            *digit = value / 2;
            remainder = value % 2;
        }
        bits.push(remainder == 1);
        while digits.len() > 1 && digits[0] == 0 {
            digits.remove(0);
        }
    }
    bits
}

/// Builds the `Pos` term for a positive decimal literal (`"0"` is not valid
/// input; `Pos` has no zero) as the binary `@c1`/`@cDub` chain
/// `crates/syntax/spec/pos.mcrl2` declares: `@cDub(b, p)` denotes `2*p + b`,
/// so the least-significant bit is the *outermost* `@cDub`, built up from the
/// leading (most-significant) bit's `@c1` inward.
fn pos_literal_binary(decimal: &str) -> DataExpression {
    let bits = decimal_bits_lsb_first(decimal);
    debug_assert!(
        *bits.last().expect("a Pos literal has at least one bit"),
        "the leading bit of a Pos literal is always set"
    );

    let cdub = system_symbols::c_dub();
    let mut term: DataExpression = system_symbols::c1().into();
    for &bit in bits[..bits.len() - 1].iter().rev() {
        term = DataApplication::with_args(&cdub, &[DataExpression::bool_literal(bit), term]).into();
    }
    term
}

/// The base-`2^64` digits of a non-negative decimal literal, least-significant
/// digit first, packed from its binary expansion. Always yields at least one
/// digit, so `"0"` becomes `[0]`.
fn decimal_words_lsb_first(decimal: &str) -> Vec<u64> {
    let bits = decimal_bits_lsb_first(decimal);

    let mut words: Vec<u64> = bits
        .chunks(WORD_BITS)
        .map(|chunk| {
            let mut word = 0u64;
            for (i, &bit) in chunk.iter().enumerate() {
                if bit {
                    word |= 1u64 << i;
                }
            }
            word
        })
        .collect();

    if words.is_empty() {
        words.push(0);
    }
    words
}

/// Folds the base-`2^64` digits of `decimal` into a digit chain, starting from
/// `leading` applied to the most significant digit and wrapping each successively
/// less significant digit in `concat`, an `@concat_digit` overload, which denotes `2^64 * p + w`.
///
/// This mirrors mCRL2's `sort_pos::pos` / `sort_nat::nat` construction in
/// `standard_numbers_utility.h` when `MCRL2_ENABLE_MACHINENUMBERS` is set.
fn digit_chain_literal(decimal: &str, leading: DataFunctionSymbol, concat: DataFunctionSymbol) -> DataExpression {
    let words = decimal_words_lsb_first(decimal);

    let mut digits = words.iter().rev();
    let most = *digits.next().expect("there is always at least one digit");
    let mut term: DataExpression = DataApplication::with_args(&leading, &[machine_number(most)]).into();
    for &word in digits {
        term = DataApplication::with_args(&concat, &[term, machine_number(word)]).into();
    }
    term
}

/// Builds the `Pos` term for a positive decimal literal (`"0"` is not valid
/// input; `Pos` has no zero).
fn pos_literal(decimal: &str, encoding: NumberEncoding) -> DataExpression {
    match encoding {
        NumberEncoding::Binary => pos_literal_binary(decimal),
        NumberEncoding::MachineWord => digit_chain_literal(
            decimal,
            system_symbols::most_significant_digit(),
            system_symbols::concat_digit_pos(),
        ),
    }
}

/// Builds the `Nat` term for a decimal literal. In the binary encoding this is
/// `@c0` for `"0"` and otherwise `@cNat` wrapping the `Pos` term; in the
/// machine-word encoding it is a digit chain, with zero represented as the
/// single digit `@most_significant_digitNat(0)` rather than `@c0`.
fn nat_literal(decimal: &str, encoding: NumberEncoding) -> DataExpression {
    match encoding {
        NumberEncoding::Binary => {
            if decimal == "0" {
                system_symbols::c0().into()
            } else {
                DataApplication::with_args(&system_symbols::c_nat(), &[pos_literal_binary(decimal)]).into()
            }
        }
        NumberEncoding::MachineWord => digit_chain_literal(
            decimal,
            system_symbols::most_significant_digit_nat(),
            system_symbols::concat_digit_nat(),
        ),
    }
}

/// Builds the `Int` term for a decimal literal. A `Number` node is always a
/// non-negative decimal string (mCRL2 has no negative numeral syntax;
/// negation is the unary `-` operator applied afterwards), so this is always
/// `@cInt`, never `@cNeg`. Both encodings share the `@cInt` constructor.
fn int_literal(decimal: &str, encoding: NumberEncoding) -> DataExpression {
    DataApplication::with_args(&system_symbols::c_int(), &[nat_literal(decimal, encoding)]).into()
}

/// Builds the `Real` term for a decimal literal: `@cReal(n, 1)`, matching
/// `Int2Real`'s equation in `crates/syntax/spec/real.mcrl2` (and `real64.mcrl2`,
/// which declares `@cReal` identically).
fn real_literal(decimal: &str, encoding: NumberEncoding) -> DataExpression {
    DataApplication::with_args(
        &system_symbols::c_real(),
        &[int_literal(decimal, encoding), pos_literal("1", encoding)],
    )
    .into()
}

/// Builds the aterm literal for a `DataExpr::Number` node whose *own*
/// inferred sort is `sort` (`Pos`/`Nat`/`Int`/`Real`) — no coercion is
/// inserted here, so the caller must have already established that this is
/// the literal's minimal inferred sort, not a wider one it is later upcast to.
pub(crate) fn lower_number_literal(decimal: &str, sort: Sort, encoding: NumberEncoding) -> DataExpression {
    match sort {
        Sort::Pos => pos_literal(decimal, encoding),
        Sort::Nat => nat_literal(decimal, encoding),
        Sort::Int => int_literal(decimal, encoding),
        Sort::Real => real_literal(decimal, encoding),
        Sort::Bool => unreachable!("a Number literal never infers to Bool"),
    }
}

/// The result of lowering one equation.
pub(crate) struct LoweredEquation {
    pub(crate) condition: Option<DataExpression>,
    pub(crate) lhs: DataExpression,
    pub(crate) rhs: DataExpression,
}

/// Re-walks one equation's condition/left/right-hand sides alongside its
/// [`EquationTyping`] — in the same canonical order inference numbered the nodes in (see
/// [`merc_typecheck::internal::for_each_expr_child`]), so each node's sort/name is found by its [`ExprId`] —
/// building `merc_data::DataExpression`s bottom-up.
///
/// Lowers variables, declared-op and builtin-op applications (including the
/// polymorphic comparison/`if` operators), numeric/boolean literals, container
/// literals, all binders (`lambda`, `forall`/`exists`, set/bag comprehensions,
/// `where`), and the numeric/container coercions widening an application
/// argument or the equation's own LHS/RHS to a shared sort. Returns `None` —
/// not an error — when a construct it does not yet cover is reached.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_equation(
    ctx: &TypeCheckContext,
    spec: &UntypedDataSpecification,
    typing: &EquationTyping,
    condition: Option<&DataExpr>,
    lhs: &DataExpr,
    rhs: &DataExpr,
    encoding: NumberEncoding,
) -> Option<LoweredEquation> {
    let EquationTyping { sorts, names } = typing;

    let mut walker = Lowering {
        ctx,
        spec,
        sorts,
        names,
        next_id: 0,
        encoding,
        literals: HashMap::new(),
    };
    let condition = match condition {
        Some(condition) => Some(walker.lower(condition)?),
        None => None,
    };

    // The equation itself joins `lhs` and `rhs` through a shared (possibly
    // wider) sort, exactly like an application's argument against its
    // parameter (see `Lowering::lower_application`): capture each side's own
    // id before coercing it, so the narrower side is coerced up to the wider
    // one rather than silently producing an ill-sorted equation.
    let lhs_id = walker.peek_id();
    let lhs = walker.lower(lhs)?;
    let rhs_id = walker.peek_id();
    let rhs = walker.lower(rhs)?;
    walker.debug_assert_walked_all();
    let lhs_sort = sorts[*lhs_id];
    let rhs_sort = sorts[*rhs_id];
    let (lhs, rhs) = match ctx.sorts.partial_cmp(lhs_sort, rhs_sort)? {
        Ordering::Equal => (lhs, rhs),
        Ordering::Less => (walker.coerce(lhs_id, lhs, lhs_sort, rhs_sort)?, rhs),
        Ordering::Greater => (lhs, walker.coerce(rhs_id, rhs, rhs_sort, lhs_sort)?),
    };

    Some(LoweredEquation { condition, lhs, rhs })
}

/// Lowers the type checked standalone expression `expr` of `data` to the aterm form, so it can
/// be handed straight to a rewriter built from [`lower_data_specification`] of `data`.
///
/// # Panics
///
/// When lowering does not support a construct that type checking accepted, which is an internal
/// error.
pub fn lower_expression(data: &DataSpecification, expr: &TypedExpression) -> DataExpression {
    lower_typed_expression(
        context(data),
        data.data_specification(),
        expression_typing(expr),
        expr.expression(),
        data.number_encoding(),
    )
    .unwrap_or_else(|| {
        panic!(
            "expression '{}' passed inference but failed lowering",
            expr.expression()
        )
    })
}

/// Re-walks one standalone expression alongside its [`EquationTyping`], the
/// counterpart of [lower_equation] for an expression typed on its own by
/// `infer_expression` (see [`DataSpecification::typecheck_expression`]).
pub(crate) fn lower_typed_expression(
    ctx: &TypeCheckContext,
    spec: &UntypedDataSpecification,
    typing: &EquationTyping,
    expr: &DataExpr,
    encoding: NumberEncoding,
) -> Option<DataExpression> {
    let EquationTyping { sorts, names } = typing;

    let mut walker = Lowering {
        ctx,
        spec,
        sorts,
        names,
        next_id: 0,
        encoding,
        literals: HashMap::new(),
    };
    let lowered = walker.lower(expr)?;
    walker.debug_assert_walked_all();
    Some(lowered)
}

struct Lowering<'a> {
    ctx: &'a TypeCheckContext,
    spec: &'a UntypedDataSpecification,
    sorts: &'a [ResolvedSortId],
    names: &'a HashMap<ExprId, NameTarget>,
    /// The [ExprId] of the next node [Self::lower] visits; every `lower` call recurses in
    /// [`merc_typecheck::internal::for_each_expr_child`]'s order, so this counter reproduces inference's numbering.
    next_id: usize,
    /// How numeric literals and numeric coercions are represented.
    encoding: NumberEncoding,
    /// The decimal text of every bare `Number` node walked so far, keyed by its
    /// [ExprId], so [Self::coerce] can rebuild a widened literal at its target
    /// sort instead of wrapping it (see there).
    literals: HashMap<ExprId, String>,
}

impl Lowering<'_> {
    /// The [ExprId] the next [Self::lower] call assigns to its expression.
    fn peek_id(&self) -> ExprId {
        ExprId::new(self.next_id)
    }

    /// Checks, once every root is lowered, that the walk assigned exactly the ids inference
    /// numbered: a walk that skipped or revisited a node would otherwise read another node's sort
    /// without failing.
    fn debug_assert_walked_all(&self) {
        debug_assert_eq!(
            self.next_id,
            self.sorts.len(),
            "lowering visited a different number of nodes than inference numbered; the walk order \
             diverged from for_each_expr_child"
        );
    }

    /// Lowers `expr`, or `None` the moment an unsupported construct is reached (see
    /// [lower_equation]).
    fn lower(&mut self, expr: &DataExpr) -> Option<DataExpression> {
        let id = self.peek_id();
        self.next_id += 1;
        let sort = self.sorts[*id];

        match &expr.node {
            // `Resolved` occurs only in a process/PBES-body expression, never in a data
            // specification's own equations (this lowering's only input), but is grouped with
            // `Id` for exhaustiveness.
            DataExprKind::Id(name) | DataExprKind::Resolved(name, _) => self.lower_id(id, name, sort),
            DataExprKind::Number(value) => {
                self.literals.insert(id, value.clone());
                self.lower_number(sort, value)
            }
            DataExprKind::Bool(value) => Some(DataExpression::bool_literal(*value)),
            DataExprKind::Application { function, arguments } => self.lower_application(sort, function, arguments),
            DataExprKind::EmptyList => Some(self.lower_empty_container(sort, ComplexSort::List)),
            DataExprKind::EmptySet => Some(self.lower_empty_container(sort, ComplexSort::FSet)),
            DataExprKind::EmptyBag => Some(self.lower_empty_container(sort, ComplexSort::FBag)),
            DataExprKind::Set(members) => self.lower_set(sort, members),
            DataExprKind::Bag(members) => self.lower_bag(sort, members),
            DataExprKind::SetBagComp { variable, predicate } => self.lower_setbagcomp(sort, variable, predicate),
            DataExprKind::Lambda { variables, body } => self.lower_lambda(sort, variables, body),
            DataExprKind::Quantifier { op, variables, body } => self.lower_quantifier(*op, variables, body),
            DataExprKind::Whr { expr, assignments } => self.lower_whr(expr, assignments),
            DataExprKind::List(_)
            | DataExprKind::Unary { .. }
            | DataExprKind::Binary { .. }
            | DataExprKind::FunctionUpdate { .. } => {
                unreachable!("lower.rs already rewrote this expression form before inference ran")
            }
        }
    }

    /// Widens the term lowered from node `id` from `from` to `to` along the
    /// sub-sort lattice, inserting the constructor chain (`@cNat`/`@cInt`/
    /// `@cReal` composed for the number lattice, `@set(@false_, _)`/
    /// `@bag(@zero_, _)` for the container lattice — see
    /// [numeric_coerce]/[container_coerce]) — or returning `term` unchanged
    /// when the two sorts already coincide. Returns `None` unless `from` is
    /// `to` or a strict subsort of it that lowering can actually build a
    /// coercion for (checked via [merc_typecheck::internal::SortInterner::is_materializable],
    /// not [merc_typecheck::internal::SortInterner::partial_cmp] alone — the relation now also
    /// relates pairs, such as container-element covariance, that have no
    /// coercion below to build); a `Def` sort has no coercion either way.
    ///
    /// A bare number literal is the exception: rather than widening the term it
    /// already produced, the literal is *rebuilt* at `to` (see
    /// [lower_number_literal]), because that is the term the mCRL2 toolset's own
    /// type checker produces — it types a numeral directly at the sort its
    /// context expects instead of upcasting the minimal one. The two agree
    /// under [`NumberEncoding::Binary`], where the widening chain is literally
    /// how a wider literal is built (`@cNat(@c1)` either way), but not under
    /// [`NumberEncoding::MachineWord`], where a digit chain has no `@cNat`
    /// constructor and the coercion would emit a `Pos2Nat` *mapping* call
    /// around a `Pos` chain instead of the `Nat` chain the literal denotes.
    fn coerce(
        &self,
        id: ExprId,
        term: DataExpression,
        from: ResolvedSortId,
        to: ResolvedSortId,
    ) -> Option<DataExpression> {
        if from == to {
            return Some(term);
        }
        if !self.ctx.sorts.is_materializable(from, to) {
            return None;
        }

        match (self.ctx.sorts.get(from), self.ctx.sorts.get(to)) {
            (ResolvedSort::Primitive(_), ResolvedSort::Primitive(to_sort)) if self.literals.contains_key(&id) => {
                let decimal = &self.literals[&id];
                Some(lower_number_literal(decimal, *to_sort, self.encoding))
            }
            (ResolvedSort::Primitive(from_sort), ResolvedSort::Primitive(to_sort)) => {
                Some(numeric_coerce(term, *from_sort, *to_sort, self.encoding))
            }
            (ResolvedSort::Container { op, subsort }, ResolvedSort::Container { .. }) => {
                let element = lower_sort(self.ctx, self.spec, *subsort);
                Some(container_coerce(term, *op, element))
            }
            (
                ResolvedSort::Function {
                    domain,
                    range: from_range,
                },
                ResolvedSort::Function { range: to_range, .. },
            ) => {
                let (domain, from_range, to_range) = (domain.clone(), *from_range, *to_range);
                let vars: Vec<DataVariable> = domain
                    .iter()
                    .enumerate()
                    .map(|(i, &sort)| {
                        DataVariable::with_sort(
                            format!("@eta{i}").as_str(),
                            lower_sort(self.ctx, self.spec, sort).copy(),
                        )
                    })
                    .collect();
                let args: Vec<DataExpression> = vars.iter().map(|v| v.clone().into()).collect();
                let applied = DataApplication::with_args(&term, &args).into();
                let coerced = self.coerce(id, applied, from_range, to_range)?;
                Some(DataAbstraction::new(BinderType::Lambda, &vars, coerced).into())
            }
            _ => None,
        }
    }

    fn lower_id(&self, id: ExprId, name: &str, sort: ResolvedSortId) -> Option<DataExpression> {
        match self.names.get(&id)? {
            NameTarget::Variable => {
                Some(DataVariable::with_sort(name, lower_sort(self.ctx, self.spec, sort).copy()).into())
            }
            NameTarget::Op { .. } | NameTarget::Builtin => {
                Some(DataFunctionSymbol::with_sort(name, lower_sort(self.ctx, self.spec, sort).copy()).into())
            }
        }
    }

    fn lower_number(&self, sort: ResolvedSortId, value: &str) -> Option<DataExpression> {
        let ResolvedSort::Primitive(sort) = self.ctx.sorts.get(sort) else {
            unreachable!("a Number literal always infers to a primitive numeric sort")
        };
        Some(lower_number_literal(value, *sort, self.encoding))
    }

    fn lower_application(
        &mut self,
        sort: ResolvedSortId,
        function: &DataExpr,
        arguments: &[DataExpr],
    ) -> Option<DataExpression> {
        // Each argument's own id is looked up before lowering it, so `coerce` can tell a bare
        // number literal from a term that merely has its sort.
        let mut argument_terms = Vec::with_capacity(arguments.len());
        let mut argument_sorts = Vec::with_capacity(arguments.len());
        let mut argument_ids = Vec::with_capacity(arguments.len());
        for argument in arguments {
            let argument_id = self.peek_id();
            argument_ids.push(argument_id);
            argument_sorts.push(self.sorts[*argument_id]);
            argument_terms.push(self.lower(argument)?);
        }
        let function_sort = self.sorts[*self.peek_id()];
        let function_term = self.lower(function)?;

        let ResolvedSort::Function { domain, range } = self.ctx.sorts.get(function_sort) else {
            unreachable!("an applied expression always infers to a function sort")
        };
        debug_assert_eq!(*range, sort, "the application's own sort is the function's range");
        if domain.len() != argument_sorts.len() {
            return None;
        }
        // Each argument widens to its domain position if needed: the domain
        // is cloned first since `coerce` below needs `self.ctx` again, which
        // this `match` already borrows through `function_sort`.
        let domain = domain.clone();

        let mut coerced_terms = Vec::with_capacity(argument_terms.len());
        for (((term, arg_sort), arg_id), &dom_sort) in argument_terms
            .into_iter()
            .zip(argument_sorts)
            .zip(argument_ids)
            .zip(domain.iter())
        {
            coerced_terms.push(self.coerce(arg_id, term, arg_sort, dom_sort)?);
        }

        Some(DataApplication::with_args(&function_term, &coerced_terms).into())
    }

    /// Builds the empty-container constant for `EmptyList` / `EmptySet` / `EmptyBag`.
    /// The sort for the constant is extracted from the node's own inferred sort.
    fn lower_empty_container(&self, sort: ResolvedSortId, op: ComplexSort) -> DataExpression {
        let ResolvedSort::Container {
            subsort: element_id, ..
        } = self.ctx.sorts.get(sort)
        else {
            unreachable!("empty container always infers to a Generic sort")
        };
        let element = lower_sort(self.ctx, self.spec, *element_id);
        match op {
            ComplexSort::List => system_symbols::empty_list(&element),
            ComplexSort::FSet => system_symbols::empty_fset(&element),
            ComplexSort::FBag => system_symbols::empty_fbag(&element),
            _ => unreachable!("lower_empty_container only handles List/FSet/FBag"),
        }
        .into()
    }

    /// Lowers `{m1, m2, …}` (parsed as `FSet(S)`) to `@fset_insert(m1, @fset_insert(m2, {}))`.
    fn lower_set(&mut self, sort: ResolvedSortId, members: &[DataExpr]) -> Option<DataExpression> {
        let ResolvedSort::Container {
            subsort: element_id, ..
        } = self.ctx.sorts.get(sort)
        else {
            unreachable!("Set literal always infers to FSet(S)")
        };
        let element_id = *element_id;
        let element = lower_sort(self.ctx, self.spec, element_id);
        let fset_insert = system_symbols::fset_insert(&element);

        let empty: DataExpression = system_symbols::empty_fset(&element).into();
        let mut lowered = Vec::with_capacity(members.len());
        for member in members {
            let member_id = self.peek_id();
            let member_sort = self.sorts[*member_id];
            let member_term = self.lower(member)?;
            lowered.push((member_id, member_term, member_sort));
        }
        let mut result = empty;
        for (member_id, member_term, member_sort) in lowered.into_iter().rev() {
            let coerced = self.coerce(member_id, member_term, member_sort, element_id)?;
            result = DataApplication::with_args(&fset_insert, &[coerced, result]).into();
        }
        Some(result)
    }

    /// Lowers `{e1:m1, e2:m2, …}` (parsed as `FBag(S)`) to
    /// `@fbag_cinsert(e1, m1, @fbag_cinsert(e2, m2, {:}))`.
    fn lower_bag(&mut self, sort: ResolvedSortId, members: &[BagElement]) -> Option<DataExpression> {
        let ResolvedSort::Container {
            subsort: element_id, ..
        } = self.ctx.sorts.get(sort)
        else {
            unreachable!("Bag literal always infers to FBag(S)")
        };
        let element_id = *element_id;
        let nat_id = self.ctx.sorts.nat_sort();
        let element = lower_sort(self.ctx, self.spec, element_id);
        let fbag_cinsert = system_symbols::fbag_cinsert(&element);

        let empty: DataExpression = system_symbols::empty_fbag(&element).into();
        let mut lowered = Vec::with_capacity(members.len());
        for member in members {
            let elem_id = self.peek_id();
            let elem_sort = self.sorts[*elem_id];
            let elem_term = self.lower(&member.expr)?;
            let mult_id = self.peek_id();
            let mult_sort = self.sorts[*mult_id];
            let mult_term = self.lower(&member.multiplicity)?;
            lowered.push((elem_id, elem_term, elem_sort, mult_id, mult_term, mult_sort));
        }
        let mut result = empty;
        for (elem_id, elem_term, elem_sort, mult_id, mult_term, mult_sort) in lowered.into_iter().rev() {
            let coerced_elem = self.coerce(elem_id, elem_term, elem_sort, element_id)?;
            let coerced_mult = self.coerce(mult_id, mult_term, mult_sort, nat_id)?;
            result = DataApplication::with_args(&fbag_cinsert, &[coerced_elem, coerced_mult, result]).into();
        }
        Some(result)
    }

    fn lower_lambda(
        &mut self,
        sort: ResolvedSortId,
        variables: &[merc_syntax::IdDecl],
        body: &DataExpr,
    ) -> Option<DataExpression> {
        let vars: Vec<DataVariable> = variables
            .iter()
            .map(|v| DataVariable::with_sort(v.identifier.as_str(), lower_syntax_sort(&v.sort).copy()))
            .collect();
        let body_id = self.peek_id();
        let body_sort = self.sorts[*body_id];
        let body_term = self.lower(body)?;
        // The lambda's own range may be wider than the body's inferred sort
        // (see the `Lambda` case in `ConstraintGenerator::visit`): the body
        // is coerced up to it here, the one place a function value's range
        // can still be widened at the term level.
        let ResolvedSort::Function { range, .. } = self.ctx.sorts.get(sort) else {
            unreachable!("a lambda always infers to a function sort")
        };
        let range = *range;
        let body = self.coerce(body_id, body_term, body_sort, range)?;
        Some(DataAbstraction::new(BinderType::Lambda, &vars, body).into())
    }

    fn lower_quantifier(
        &mut self,
        op: Quantifier,
        variables: &[merc_syntax::IdDecl],
        body: &DataExpr,
    ) -> Option<DataExpression> {
        let binder = match op {
            Quantifier::Forall => BinderType::Forall,
            Quantifier::Exists => BinderType::Exists,
        };

        let vars: Vec<DataVariable> = variables
            .iter()
            .map(|v| DataVariable::with_sort(v.identifier.as_str(), lower_syntax_sort(&v.sort).copy()))
            .collect();

        let body = self.lower(body)?;
        Some(DataAbstraction::new(binder, &vars, body).into())
    }

    /// Lowers `{ x: S | e }` to the `Set`/`Bag` constructor applied to the
    /// characteristic function `lambda x: S. e` and the empty finite
    /// set/bag — `@set(lambda x: S. e, {})` / `@bag(lambda x: S. e, {:})`.
    ///
    /// This is the form the mCRL2 toolset's type checker produces: a `Set(S)`
    /// *is* a `(S -> Bool) # FSet(S)` pair (`set.mcrl2`), so a comprehension has
    /// no representation of its own in the lowered term — the dedicated
    /// `SetComp`/`BagComp` binder kinds exist in the aterm schema but are not
    /// what a checked specification holds.
    ///
    /// A bag comprehension's body counts multiplicities, so a `Pos` body is
    /// widened to the `Nat` the `@bag` multiplicity function requires (the two
    /// readings of a comprehension are what `Comprehension` in `inference.rs`
    /// resolves).
    fn lower_setbagcomp(
        &mut self,
        sort: ResolvedSortId,
        variable: &merc_syntax::IdDecl,
        predicate: &DataExpr,
    ) -> Option<DataExpression> {
        let (op, element_id) = match self.ctx.sorts.get(sort) {
            ResolvedSort::Container { op, subsort } => (*op, *subsort),
            _ => unreachable!("SetBagComp always infers to Set or Bag"),
        };
        let element = lower_sort(self.ctx, self.spec, element_id);
        let var = DataVariable::with_sort(variable.identifier.as_str(), element.copy());

        let body_id = self.peek_id();
        let body_sort = self.sorts[*body_id];
        let body = self.lower(predicate)?;

        let body = match op {
            ComplexSort::Set => body,
            ComplexSort::Bag => self.coerce(body_id, body, body_sort, self.ctx.sorts.nat_sort())?,
            _ => unreachable!("SetBagComp infers only to Set or Bag"),
        };
        let characteristic: DataExpression = DataAbstraction::new(BinderType::Lambda, &[var], body).into();

        let (constructor, empty) = match op {
            ComplexSort::Set => (
                system_symbols::set_constructor(&element),
                system_symbols::empty_fset(&element),
            ),
            _ => (
                system_symbols::bag_constructor(&element),
                system_symbols::empty_fbag(&element),
            ),
        };

        Some(DataApplication::with_args(&constructor, &[characteristic, empty.into()]).into())
    }

    fn lower_whr(&mut self, expr: &DataExpr, assignments: &[merc_syntax::Assignment]) -> Option<DataExpression> {
        let mut whr_decls = Vec::with_capacity(assignments.len());
        for assignment in assignments {
            let assignment_sort = self.sorts[*self.peek_id()];
            let assignment_term = self.lower(&assignment.expr)?;
            let var = DataVariable::with_sort(
                assignment.identifier.as_str(),
                lower_sort(self.ctx, self.spec, assignment_sort).copy(),
            );
            whr_decls.push(DataWhrDecl::new(var, assignment_term));
        }
        let body = self.lower(expr)?;
        Some(DataWhereClause::new(body, &whr_decls).into())
    }
}

/// Converts a (normalized, desugared) `merc_syntax` sort expression into the
/// `merc_data` sort term the mCRL2 binary schema uses.
///
/// Handles every form left after the `from_untyped` pipeline:
/// `Simple` → `BasicSort`, `Complex` → `SortCons`, `FlattenedFunction` and
/// `Function` (the system spec is not flattened) → `SortArrow`, `Resolved` and
/// `Reference` → `BasicSort` by name. `Struct` and a bare `Product` are
/// unreachable at this point.
pub(crate) fn lower_syntax_sort(sort: &SortExpression) -> DataSortExpression {
    match &sort.node {
        SortExpressionKind::Simple(s) => BasicSort::new(s.name()).into(),
        SortExpressionKind::Complex(op, sub) => SortCons::new(container_kind(*op), lower_syntax_sort(sub)).into(),
        SortExpressionKind::FlattenedFunction { domain, range } => {
            let domain: Vec<DataSortExpression> = domain.iter().map(lower_syntax_sort).collect();
            SortArrow::new(&domain, &lower_syntax_sort(range)).into()
        }
        SortExpressionKind::Function { domain, range } => {
            // The system spec is not flattened; flatten the Product spine here.
            let mut flat = Vec::new();
            flatten_product_domain(domain, &mut flat);
            SortArrow::new(&flat, &lower_syntax_sort(range)).into()
        }
        // A user-declared or struct-representative sort after name resolution,
        // or an unresolved template reference in the system spec (e.g. "S", "T").
        // Both use the string name — the identity of a nominal sort IS its name
        // in the binary schema.
        SortExpressionKind::Resolved(name, _) | SortExpressionKind::Reference(name) => {
            BasicSort::new(name.as_str()).into()
        }
        SortExpressionKind::TypeVar(_) | SortExpressionKind::ResolvedTypeVar(_) => unreachable!(
            "no TypeVar/ResolvedTypeVar node reaches lowering: the container/function-update \
             templates do declare their own sort variable(s) with a `type_var` block now (see the \
             unifying-polymorphism design), but `replace_sort` always substitutes every \
             ResolvedTypeVar node for a concrete sort before the result is merged into `system`, and \
             a scheme reached through inference is instantiated (see template_instance) before its \
             result is ever lowered"
        ),
        SortExpressionKind::Struct { .. } | SortExpressionKind::Product { .. } => {
            unreachable!("struct/product sorts are desugared/flattened before lowering")
        }
    }
}

fn flatten_product_domain(sort: &SortExpression, domain: &mut Vec<DataSortExpression>) {
    match &sort.node {
        SortExpressionKind::Product { lhs, rhs } => {
            flatten_product_domain(lhs, domain);
            flatten_product_domain(rhs, domain);
        }
        _ => domain.push(lower_syntax_sort(sort)),
    }
}

// ──────────────────────── lower_data_specification ───────────────────────────

/// Assembles a [`Mcrl2DataSpecification`] from the already-type-checked user
/// and system specifications of `data`:
///
/// - **sorts** — user abstract sorts (those whose declaration has no right-hand
///   side after desugaring and normalization).
/// - **aliases** — user sort aliases (those that do have a right-hand side).
/// - **constructors / mappings** — user declarations lowered via the interned
///   sort lattice, followed by system declarations lowered directly from their
///   syntax sorts (the system spec is deliberately left unresolved).
/// - **equations** — user equations, lowered via their
///   [`EquationTyping`] and `lower_equation`, followed by system-defined
///   equations lowered the same way from `ctx.system_equation_typing`.
pub fn lower_data_specification(data: &DataSpecification) -> Mcrl2DataSpecification {
    let ctx = context(data);
    let spec = data.data_specification();
    let system = data.system_defined_specification();
    let encoding = data.number_encoding();

    // `@NatPair`/`@word` share `spec.sort_declarations` with the user's own sorts (folded in by
    // `DataSpecification::from_untyped_with` so they get a real `SortId` from the same pass), but
    // the lowered aterm's own `sorts()` must stay exactly what the user declared: the mCRL2 toolset
    // never declares them as a `sort` in its own output either, treating them as an implementation
    // detail baked into `Nat`/`@word`'s own constructor and mapping signatures instead. Told apart
    // by the reserved `@`-name convention system-generated declarations use, the same one
    // `typing_info::sort_declaration_by_id` relies on.
    let sorts: Vec<BasicSort> = spec
        .sort_declarations
        .iter()
        .filter(|d| d.expr.is_none() && !is_system_generated_name(&d.identifier))
        .map(|d| BasicSort::new(d.identifier.as_str()))
        .collect();

    let aliases: Vec<SortAlias> = spec
        .sort_declarations
        .iter()
        .filter_map(|d| {
            let expr = d.expr.as_ref()?;
            Some(SortAlias::new(
                BasicSort::new(d.identifier.as_str()),
                lower_syntax_sort(expr),
            ))
        })
        .collect();

    let mut constructors: Vec<DataFunctionSymbol> = spec
        .constructor_declarations
        .iter()
        .map(|decl| {
            let id = decl.id.expect("assign_declaration_ids ran before lowering");
            // `build_signature` resolved and interned every declaration sort
            // during `from_untyped`, so the sort is already in the context;
            // reading it keeps lowering an immutable pass over the context.
            let sort_id = ctx
                .sort_of_constructor
                .get(&id)
                .copied()
                .expect("constructor sorts are all resolved during from_untyped");
            DataFunctionSymbol::with_sort(decl.identifier.as_str(), lower_sort(ctx, spec, sort_id).copy())
        })
        .collect();
    for decl in &system.constructor_declarations {
        constructors.push(DataFunctionSymbol::with_sort(
            decl.identifier.as_str(),
            lower_syntax_sort(&decl.sort).copy(),
        ));
    }

    let mut mappings: Vec<DataFunctionSymbol> = spec
        .map_declarations
        .iter()
        .map(|decl| {
            let id = decl.id.expect("assign_declaration_ids ran before lowering");
            let sort_id = ctx
                .sort_of_map
                .get(&id)
                .copied()
                .expect("map sorts are all resolved during from_untyped");
            DataFunctionSymbol::with_sort(decl.identifier.as_str(), lower_sort(ctx, spec, sort_id).copy())
        })
        .collect();
    for decl in &system.map_declarations {
        mappings.push(DataFunctionSymbol::with_sort(
            decl.identifier.as_str(),
            lower_syntax_sort(&decl.sort).copy(),
        ));
    }

    let mut equations: Vec<DataEquation> = Vec::new();
    for eqn_spec in &spec.equation_declarations {
        let eqn_spec_id = eqn_spec.id.expect("assign_declaration_ids ran before lowering");
        let vars: Vec<DataVariable> = eqn_spec
            .variables
            .iter()
            .map(|var| DataVariable::with_sort(var.identifier.as_str(), lower_syntax_sort(&var.sort).copy()))
            .collect();
        for eqn in &eqn_spec.equations {
            let equation_id = eqn.id.expect("assign_declaration_ids ran before lowering");
            // `check_equations` inferred and cached every user equation during
            // `from_untyped`, so the typing is read straight from the context —
            // the same immutable-read pattern as the constructor/map sorts above.
            let typing = ctx
                .equation_typing
                .get(&(eqn_spec_id, equation_id))
                .expect("equation typings are all resolved during from_untyped")
                .as_ref()
                .expect("a well-typed specification has no equation inference errors");
            // Sort inference already accepted this equation, so `None` means `Lowering`
            // is missing a construct it supports: an internal bug, and a hard
            // failure rather than a silently dropped rewrite rule.
            let lowered = lower_equation(ctx, spec, typing, eqn.condition.as_ref(), &eqn.lhs, &eqn.rhs, encoding)
                .unwrap_or_else(|| panic!("user equation '{eqn}' passed sort inference but failed lowering"));
            equations.push(DataEquation::new(&vars, lowered.condition, lowered.lhs, lowered.rhs));
        }
    }

    for eqn_spec in &system.equation_declarations {
        let eqn_spec_id = eqn_spec
            .id
            .expect("assign_declaration_ids ran on system before lowering");
        let vars: Vec<DataVariable> = eqn_spec
            .variables
            .iter()
            .map(|var| DataVariable::with_sort(var.identifier.as_str(), lower_syntax_sort(&var.sort).copy()))
            .collect();
        for eqn in &eqn_spec.equations {
            let equation_id = eqn.id.expect("assign_declaration_ids ran on system before lowering");
            let typing = ctx
                .system_equation_typing
                .get(&(eqn_spec_id, equation_id))
                .expect("system equation typings are all resolved during from_untyped")
                .as_ref()
                .expect("a well-typed specification has no system equation inference errors");
            let lowered = lower_equation(ctx, spec, typing, eqn.condition.as_ref(), &eqn.lhs, &eqn.rhs, encoding)
                .unwrap_or_else(|| panic!("system equation '{eqn}' passed sort inference but failed lowering"));
            equations.push(DataEquation::new(&vars, lowered.condition, lowered.lhs, lowered.rhs));
        }
    }

    // Every container/function-update/comparison instantiation the specification actually uses,
    // monomorphized for this call only. The system specification above was already lowered, so
    // this adds only the instantiations.
    let (scratch_ctx, generated) = instantiate_used_templates(data);

    for decl in &generated.constructor_declarations {
        constructors.push(DataFunctionSymbol::with_sort(
            decl.identifier.as_str(),
            lower_syntax_sort(&decl.sort).copy(),
        ));
    }
    for decl in &generated.map_declarations {
        mappings.push(DataFunctionSymbol::with_sort(
            decl.identifier.as_str(),
            lower_syntax_sort(&decl.sort).copy(),
        ));
    }

    for eqn_spec in &generated.equation_declarations {
        let eqn_spec_id = eqn_spec
            .id
            .expect("assign_declaration_ids ran on the generated content above");
        let vars: Vec<DataVariable> = eqn_spec
            .variables
            .iter()
            .map(|var| DataVariable::with_sort(var.identifier.as_str(), lower_syntax_sort(&var.sort).copy()))
            .collect();
        for eqn in &eqn_spec.equations {
            let equation_id = eqn
                .id
                .expect("assign_declaration_ids ran on the generated content above");
            let typing = scratch_ctx
                .system_equation_typing
                .get(&(eqn_spec_id, equation_id))
                .expect("instantiate_system_equations resolved every generated equation's typing above")
                .as_ref()
                .expect("a well-typed template specializes to a well-typed instantiation");
            let lowered = lower_equation(
                &scratch_ctx,
                spec,
                typing,
                eqn.condition.as_ref(),
                &eqn.lhs,
                &eqn.rhs,
                encoding,
            )
            .unwrap_or_else(|| panic!("generated equation '{eqn}' passed sort inference but failed lowering"));
            equations.push(DataEquation::new(&vars, lowered.condition, lowered.lhs, lowered.rhs));
        }
    }

    Mcrl2DataSpecification::new(sorts, aliases, constructors, mappings, equations)
}
