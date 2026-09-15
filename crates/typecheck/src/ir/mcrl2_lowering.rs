use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;

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
use merc_syntax::SourceMap;
use merc_syntax::UntypedDataSpecification;

use crate::EquationTyping;
use crate::ExprId;
use crate::NameTarget;
use crate::NumberEncoding;
use crate::ResolvedSort;
use crate::ResolvedSortId;
use crate::TypeCheckContext;
use crate::assign_declaration_ids;
use crate::build_system_defined_specification;
use crate::check_multi_argument_function_update_template;
use crate::check_system_equations;
use crate::check_system_specification;
use crate::extend_system_with_inferred_sorts;
use crate::resolve_data_specification_variables;
use crate::unreachable_not_a_value_sort;

/// The mCRL2 name of a basic sort, matching the literal `SortId` names the
/// binary aterm format uses.
fn primitive_name(sort: Sort) -> &'static str {
    match sort {
        Sort::Bool => "Bool",
        Sort::Pos => "Pos",
        Sort::Nat => "Nat",
        Sort::Int => "Int",
        Sort::Real => "Real",
    }
}

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
                NumberEncoding::Binary => function_symbol("@cNat", &[pos_sort()], nat_sort()),
                NumberEncoding::MachineWord => function_symbol("Pos2Nat", &[pos_sort()], nat_sort()),
            };
            (DataApplication::with_args(&convert, &[term]).into(), Sort::Nat)
        }
        Sort::Nat => {
            let cint = function_symbol("@cInt", &[nat_sort()], int_sort());
            (DataApplication::with_args(&cint, &[term]).into(), Sort::Int)
        }
        Sort::Int => {
            let creal = function_symbol("@cReal", &[int_sort(), pos_sort()], real_sort());
            (
                DataApplication::with_args(&creal, &[term, pos_literal("1", encoding)]).into(),
                Sort::Real,
            )
        }
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
        ComplexSort::FSet => {
            let false_fn = function_symbol("@false_", std::slice::from_ref(&element), bool_sort());
            let set_sort = SortCons::new(ContainerSortKind::Set, element.clone());
            let fset_sort = SortCons::new(ContainerSortKind::FSet, element.clone());
            let predicate_sort: DataSortExpression = SortArrow::new(&[element], bool_sort()).into();
            let set_cons = function_symbol("@set", &[predicate_sort, fset_sort.into()], set_sort.into());
            DataApplication::with_args(&set_cons, &[false_fn.into(), term]).into()
        }
        ComplexSort::FBag => {
            let zero_fn = function_symbol("@zero_", std::slice::from_ref(&element), nat_sort());
            let bag_sort = SortCons::new(ContainerSortKind::Bag, element.clone());
            let fbag_sort = SortCons::new(ContainerSortKind::FBag, element.clone());
            let multiplicity_sort: DataSortExpression = SortArrow::new(&[element], nat_sort()).into();
            let bag_cons = function_symbol("@bag", &[multiplicity_sort, fbag_sort.into()], bag_sort.into());
            DataApplication::with_args(&bag_cons, &[zero_fn.into(), term]).into()
        }
        ComplexSort::List | ComplexSort::Set | ComplexSort::Bag => {
            unreachable!("only FSet and FBag widen to another container")
        }
    }
}

/// Converts an inferred, interned sort into the aterm `SortExpression` the
/// binary format uses: `Primitive`/`Generic`/`Function` recurse structurally
/// onto `BasicSort`/`SortCons`/`SortArrow`, and `Def` resolves to its declared
/// name via [TypeCheckContext::sort_display_name] — a user sort or a
/// system-internal one alike, both declared in `spec` (see
/// `docs/typecheck.md`'s `DefId`-offset milestone), or a synthesized
/// placeholder as a last resort: a nominal sort's identity *is* its declared
/// name for the binary schema.
#[allow(dead_code)]
pub(crate) fn lower_sort(
    ctx: &TypeCheckContext,
    spec: &UntypedDataSpecification,
    id: ResolvedSortId,
) -> DataSortExpression {
    match ctx.sorts.get(id) {
        ResolvedSort::Unit => unreachable_not_a_value_sort("Unit"),
        ResolvedSort::Primitive(sort) => BasicSort::new(primitive_name(*sort)).into(),
        ResolvedSort::Generic { op, subsort } => {
            SortCons::new(container_kind(*op), lower_sort(ctx, spec, *subsort)).into()
        }
        ResolvedSort::Function { domain, range } => {
            let domain: Vec<DataSortExpression> = domain.iter().map(|&sort| lower_sort(ctx, spec, sort)).collect();
            SortArrow::new(&domain, lower_sort(ctx, spec, *range)).into()
        }
        ResolvedSort::Def(def) => BasicSort::new(ctx.sort_display_name(spec, *def).as_ref()).into(),
        ResolvedSort::Var(_) => unreachable_not_a_value_sort("Var"),
    }
}

fn pos_sort() -> DataSortExpression {
    BasicSort::new("Pos").into()
}

fn nat_sort() -> DataSortExpression {
    BasicSort::new("Nat").into()
}

fn int_sort() -> DataSortExpression {
    BasicSort::new("Int").into()
}

fn real_sort() -> DataSortExpression {
    BasicSort::new("Real").into()
}

fn bool_sort() -> DataSortExpression {
    BasicSort::new("Bool").into()
}

/// The sort of a machine word, the digit sort of [`NumberEncoding::MachineWord`]
/// (declared by `crates/syntax/spec/machine_word.mcrl2`).
fn word_sort() -> DataSortExpression {
    BasicSort::new("@word").into()
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

fn bool_literal(value: bool) -> DataExpression {
    constant(if value { "true" } else { "false" }, bool_sort()).into()
}

/// Builds a nullary function symbol (constructor/constant) of `sort`.
fn constant(name: &str, sort: DataSortExpression) -> DataFunctionSymbol {
    DataFunctionSymbol::with_sort(name, sort.copy())
}

/// Builds a function symbol of `domain -> range`.
fn function_symbol(name: &str, domain: &[DataSortExpression], range: DataSortExpression) -> DataFunctionSymbol {
    let sort: DataSortExpression = SortArrow::new(domain, range).into();
    DataFunctionSymbol::with_sort(name, sort.copy())
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

    let cdub = function_symbol("@cDub", &[bool_sort(), pos_sort()], pos_sort());
    let mut term: DataExpression = constant("@c1", pos_sort()).into();
    for &bit in bits[..bits.len() - 1].iter().rev() {
        term = DataApplication::with_args(&cdub, &[bool_literal(bit), term]).into();
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
/// `most_significant` applied to the leading digit and wrapping each successively
/// less significant digit in `@concat_digit`, which denotes `2^64 * p + w`.
///
/// This mirrors mCRL2's `sort_pos::pos` / `sort_nat::nat` construction in
/// `standard_numbers_utility.h` when `MCRL2_ENABLE_MACHINENUMBERS` is set.
fn digit_chain_literal(decimal: &str, most_significant: &str, sort: DataSortExpression) -> DataExpression {
    let words = decimal_words_lsb_first(decimal);

    let leading = function_symbol(most_significant, &[word_sort()], sort.clone());
    let concat = function_symbol("@concat_digit", &[sort.clone(), word_sort()], sort);

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
        NumberEncoding::MachineWord => digit_chain_literal(decimal, "@most_significant_digit", pos_sort()),
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
                constant("@c0", nat_sort()).into()
            } else {
                let cnat = function_symbol("@cNat", &[pos_sort()], nat_sort());
                DataApplication::with_args(&cnat, &[pos_literal_binary(decimal)]).into()
            }
        }
        NumberEncoding::MachineWord => digit_chain_literal(decimal, "@most_significant_digitNat", nat_sort()),
    }
}

/// Builds the `Int` term for a decimal literal. A `Number` node is always a
/// non-negative decimal string (mCRL2 has no negative numeral syntax;
/// negation is the unary `-` operator applied afterwards), so this is always
/// `@cInt`, never `@cNeg`. Both encodings share the `@cInt` constructor.
fn int_literal(decimal: &str, encoding: NumberEncoding) -> DataExpression {
    let cint = function_symbol("@cInt", &[nat_sort()], int_sort());
    DataApplication::with_args(&cint, &[nat_literal(decimal, encoding)]).into()
}

/// Builds the `Real` term for a decimal literal: `@cReal(n, 1)`, matching
/// `Int2Real`'s equation in `crates/syntax/spec/real.mcrl2` (and `real64.mcrl2`,
/// which declares `@cReal` identically).
fn real_literal(decimal: &str, encoding: NumberEncoding) -> DataExpression {
    let creal = function_symbol("@cReal", &[int_sort(), pos_sort()], real_sort());
    DataApplication::with_args(&creal, &[int_literal(decimal, encoding), pos_literal("1", encoding)]).into()
}

/// Builds the aterm literal for a `DataExpr::Number` node whose *own*
/// inferred sort is `sort` (`Pos`/`Nat`/`Int`/`Real`) — no coercion is
/// inserted here, so the caller must have already established that this is
/// the literal's minimal inferred sort, not a wider one it is later upcast to.
#[allow(dead_code)]
pub(crate) fn lower_number_literal(decimal: &str, sort: Sort, encoding: NumberEncoding) -> DataExpression {
    match sort {
        Sort::Pos => pos_literal(decimal, encoding),
        Sort::Nat => nat_literal(decimal, encoding),
        Sort::Int => int_literal(decimal, encoding),
        Sort::Real => real_literal(decimal, encoding),
        Sort::Bool => unreachable!("a Number literal never infers to Bool"),
    }
}

/// Builds the aterm literal for a `DataExpr::Bool` node.
#[allow(dead_code)]
pub(crate) fn lower_bool_literal(value: bool) -> DataExpression {
    bool_literal(value)
}

/// The result of lowering one equation.
#[allow(dead_code)]
pub(crate) struct LoweredEquation {
    pub(crate) condition: Option<DataExpression>,
    pub(crate) lhs: DataExpression,
    pub(crate) rhs: DataExpression,
}

/// Re-walks one equation's condition/left/right-hand sides alongside its
/// [`EquationTyping`] side tables, in the exact `ExprId` order generation used
/// (documented on `ExprId` in inference.rs: parents before children, arguments
/// before the applied function), building `merc_data::DataExpression`s
/// bottom-up.
///
/// Lowers variables, declared-op and builtin-op applications (including the
/// polymorphic comparison/`if` operators), numeric/boolean literals, container
/// literals, all binders (`lambda`, `forall`/`exists`, set/bag comprehensions,
/// `where`), and the numeric/container coercions widening an application
/// argument or the equation's own LHS/RHS to a shared sort. Returns `None` —
/// not an error — when a construct it does not yet cover is reached.
#[allow(dead_code)]
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
    let EquationTyping { sorts, names, .. } = typing;

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
    // id *before* lowering it, so the narrower side is coerced up to the
    // wider one rather than silently producing an ill-sorted equation.
    let lhs_id = ExprId::new(walker.next_id);
    let lhs = walker.lower(lhs)?;
    let rhs_id = ExprId::new(walker.next_id);
    let rhs = walker.lower(rhs)?;
    let lhs_sort = sorts[*lhs_id];
    let rhs_sort = sorts[*rhs_id];
    let (lhs, rhs) = match ctx.sorts.partial_cmp(lhs_sort, rhs_sort)? {
        Ordering::Equal => (lhs, rhs),
        Ordering::Less => (walker.coerce(lhs_id, lhs, lhs_sort, rhs_sort)?, rhs),
        Ordering::Greater => (lhs, walker.coerce(rhs_id, rhs, rhs_sort, lhs_sort)?),
    };

    Some(LoweredEquation { condition, lhs, rhs })
}

/// Re-walks one standalone expression alongside its [`EquationTyping`], the
/// counterpart of [lower_equation] for an expression typed on its own by
/// `infer_expression` (see [`crate::DataSpecification::typecheck_expression`]).
///
/// The `ExprId` numbering of a lone expression starts at its own root, so the
/// walk is the same one [lower_equation] performs on an equation side.
pub(crate) fn lower_expression(
    ctx: &TypeCheckContext,
    spec: &UntypedDataSpecification,
    typing: &EquationTyping,
    expr: &DataExpr,
    encoding: NumberEncoding,
) -> Option<DataExpression> {
    let EquationTyping { sorts, names, .. } = typing;

    Lowering {
        ctx,
        spec,
        sorts,
        names,
        next_id: 0,
        encoding,
        literals: HashMap::new(),
    }
    .lower(expr)
}

struct Lowering<'a> {
    ctx: &'a TypeCheckContext,
    spec: &'a UntypedDataSpecification,
    sorts: &'a [ResolvedSortId],
    names: &'a HashMap<ExprId, NameTarget>,
    /// The `ExprId` the next node visited will be assigned, mirroring
    /// `ConstraintGenerator::visit`'s `id = ExprId::new(self.expr_sorts.len())`.
    next_id: usize,
    /// How numeric literals and numeric coercions are represented.
    encoding: NumberEncoding,
    /// The decimal text of every bare `Number` node walked so far, keyed by its
    /// [ExprId], so [Self::coerce] can rebuild a widened literal at its target
    /// sort instead of wrapping it (see there).
    literals: HashMap<ExprId, String>,
}

impl Lowering<'_> {
    /// Lowers `expr`, consuming exactly the `ExprId`s generation would have
    /// assigned to its subtree, or `None` the moment an unsupported
    /// construct is reached (see [lower_equation]).
    fn lower(&mut self, expr: &DataExpr) -> Option<DataExpression> {
        let id = ExprId::new(self.next_id);
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
            DataExprKind::Bool(value) => Some(lower_bool_literal(*value)),
            DataExprKind::Application { function, arguments } => self.lower_application(sort, function, arguments),
            DataExprKind::EmptyList => Some(self.lower_empty_container(sort, ComplexSort::List)),
            DataExprKind::EmptySet => Some(self.lower_empty_container(sort, ComplexSort::FSet)),
            DataExprKind::EmptyBag => Some(self.lower_empty_container(sort, ComplexSort::FBag)),
            DataExprKind::Set(members) => self.lower_set(sort, members),
            DataExprKind::Bag(members) => self.lower_bag(sort, members),
            DataExprKind::SetBagComp { variable, predicate } => self.lower_setbagcomp(sort, variable, predicate),
            DataExprKind::Lambda { variables, body } => self.lower_lambda(variables, body),
            DataExprKind::Quantifier { op, variables, body } => self.lower_quantifier(op.clone(), variables, body),
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
    /// `to` or a strict subsort of it (checked via
    /// [crate::SortInterner::partial_cmp]); a `Def` sort has no coercion either
    /// way.
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
        if self.ctx.sorts.partial_cmp(from, to) != Some(Ordering::Less) {
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
            (ResolvedSort::Generic { op, subsort }, ResolvedSort::Generic { .. }) => {
                let element = lower_sort(self.ctx, self.spec, *subsort);
                Some(container_coerce(term, *op, element))
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
        // Arguments before the applied function, matching generation order.
        // Each argument's own id is captured before lowering it, so `coerce`
        // can tell a bare number literal from a term that merely has its sort.
        let mut argument_terms = Vec::with_capacity(arguments.len());
        let mut argument_sorts = Vec::with_capacity(arguments.len());
        let mut argument_ids = Vec::with_capacity(arguments.len());
        for argument in arguments {
            argument_ids.push(ExprId::new(self.next_id));
            argument_sorts.push(self.sorts[self.next_id]);
            argument_terms.push(self.lower(argument)?);
        }
        let function_sort = self.sorts[self.next_id];
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
        let ResolvedSort::Generic {
            subsort: element_id, ..
        } = self.ctx.sorts.get(sort)
        else {
            unreachable!("empty container always infers to a Generic sort")
        };
        let element = lower_sort(self.ctx, self.spec, *element_id);
        let container: DataSortExpression = SortCons::new(container_kind(op), element).into();
        let name = match op {
            ComplexSort::List => "[]",
            ComplexSort::FSet => "{}",
            ComplexSort::FBag => "{:}",
            _ => unreachable!("lower_empty_container only handles List/FSet/FBag"),
        };
        DataFunctionSymbol::with_sort(name, container.copy()).into()
    }

    /// Lowers `{m1, m2, …}` (parsed as `FSet(S)`) to `@fset_insert(m1, @fset_insert(m2, {}))`.
    fn lower_set(&mut self, sort: ResolvedSortId, members: &[DataExpr]) -> Option<DataExpression> {
        let ResolvedSort::Generic {
            subsort: element_id, ..
        } = self.ctx.sorts.get(sort)
        else {
            unreachable!("Set literal always infers to FSet(S)")
        };
        let element_id = *element_id;
        let element = lower_sort(self.ctx, self.spec, element_id);
        let fset: DataSortExpression = SortCons::new(ContainerSortKind::FSet, element.clone()).into();
        let fset_insert = function_symbol("@fset_insert", &[element.clone(), fset.clone()], fset.clone());

        let empty: DataExpression = DataFunctionSymbol::with_sort("{}", fset.copy()).into();
        let mut lowered = Vec::with_capacity(members.len());
        for member in members {
            let member_id = ExprId::new(self.next_id);
            let member_sort = self.sorts[self.next_id];
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
        let ResolvedSort::Generic {
            subsort: element_id, ..
        } = self.ctx.sorts.get(sort)
        else {
            unreachable!("Bag literal always infers to FBag(S)")
        };
        let element_id = *element_id;
        let nat_id = self.ctx.sorts.nat_sort();
        let element = lower_sort(self.ctx, self.spec, element_id);
        let fbag: DataSortExpression = SortCons::new(ContainerSortKind::FBag, element.clone()).into();
        let fbag_cinsert = function_symbol(
            "@fbag_cinsert",
            &[element.clone(), nat_sort(), fbag.clone()],
            fbag.clone(),
        );

        let empty: DataExpression = DataFunctionSymbol::with_sort("{:}", fbag.copy()).into();
        let mut lowered = Vec::with_capacity(members.len());
        for member in members {
            let elem_id = ExprId::new(self.next_id);
            let elem_sort = self.sorts[self.next_id];
            let elem_term = self.lower(&member.expr)?;
            let mult_id = ExprId::new(self.next_id);
            let mult_sort = self.sorts[self.next_id];
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

    fn lower_lambda(&mut self, variables: &[merc_syntax::IdDecl], body: &DataExpr) -> Option<DataExpression> {
        let vars: Vec<DataVariable> = variables
            .iter()
            .map(|v| DataVariable::with_sort(v.identifier.as_str(), lower_syntax_sort(&v.sort).copy()))
            .collect();
        let body = self.lower(body)?;
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
            ResolvedSort::Generic { op, subsort } => (*op, *subsort),
            _ => unreachable!("SetBagComp always infers to Set or Bag"),
        };
        let element = lower_sort(self.ctx, self.spec, element_id);
        let var = DataVariable::with_sort(variable.identifier.as_str(), element.copy());

        let body_id = ExprId::new(self.next_id);
        let body_sort = self.sorts[self.next_id];
        let body = self.lower(predicate)?;

        let (body, range) = match op {
            ComplexSort::Set => (body, bool_sort()),
            ComplexSort::Bag => (
                self.coerce(body_id, body, body_sort, self.ctx.sorts.nat_sort())?,
                nat_sort(),
            ),
            _ => unreachable!("SetBagComp infers only to Set or Bag"),
        };
        let characteristic: DataExpression = DataAbstraction::new(BinderType::Lambda, &[var], body).into();

        let container: DataSortExpression = SortCons::new(container_kind(op), element.clone()).into();
        let finite_kind = match op {
            ComplexSort::Set => ContainerSortKind::FSet,
            ComplexSort::Bag => ContainerSortKind::FBag,
            _ => unreachable!("SetBagComp infers only to Set or Bag"),
        };
        let finite: DataSortExpression = SortCons::new(finite_kind, element.clone()).into();
        let empty: DataExpression = DataFunctionSymbol::with_sort(
            match op {
                ComplexSort::Set => "{}",
                _ => "{:}",
            },
            finite.copy(),
        )
        .into();

        let function_sort: DataSortExpression = SortArrow::new(&[element], range).into();
        let constructor = function_symbol(
            match op {
                ComplexSort::Set => "@set",
                _ => "@bag",
            },
            &[function_sort, finite],
            container,
        );

        Some(DataApplication::with_args(&constructor, &[characteristic, empty]).into())
    }

    fn lower_whr(&mut self, expr: &DataExpr, assignments: &[merc_syntax::Assignment]) -> Option<DataExpression> {
        let mut whr_decls = Vec::with_capacity(assignments.len());
        for assignment in assignments {
            let assignment_sort = self.sorts[self.next_id];
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
        SortExpressionKind::Simple(s) => BasicSort::new(primitive_name(*s)).into(),
        SortExpressionKind::Complex(op, sub) => SortCons::new(container_kind(*op), lower_syntax_sort(sub)).into(),
        SortExpressionKind::FlattenedFunction { domain, range } => {
            let domain: Vec<DataSortExpression> = domain.iter().map(lower_syntax_sort).collect();
            SortArrow::new(&domain, lower_syntax_sort(range)).into()
        }
        SortExpressionKind::Function { domain, range } => {
            // The system spec is not flattened; flatten the Product spine here.
            let mut flat = Vec::new();
            flatten_product_domain(domain, &mut flat);
            SortArrow::new(&flat, lower_syntax_sort(range)).into()
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
/// and system specifications:
///
/// - **sorts** — user abstract sorts (those whose declaration has no right-hand
///   side after desugaring and normalization).
/// - **aliases** — user sort aliases (those that do have a right-hand side).
/// - **constructors / mappings** — user declarations lowered via the interned
///   sort lattice, followed by system declarations lowered directly from their
///   syntax sorts (the system spec is deliberately left unresolved).
/// - **equations** — user equations, lowered via their Phase-3
///   [`EquationTyping`] and [`lower_equation`], followed by system-defined
///   equations lowered the same way from `ctx.system_equation_typing`.
pub(crate) fn lower_data_specification(
    ctx: &TypeCheckContext,
    spec: &UntypedDataSpecification,
    system: &UntypedDataSpecification,
    encoding: NumberEncoding,
) -> Mcrl2DataSpecification {
    // `@NatPair`/`@word` share `spec.sort_declarations` with the user's own sorts (folded in by
    // `DataSpecification::from_untyped_with` so they get a real `SortId` from the same pass — see
    // `docs/typecheck.md`'s `DefId`-offset milestone), but the lowered aterm's own `sorts()` must
    // stay exactly what the user declared: the mCRL2 toolset never declares them as a `sort` in its
    // own output either, treating them as an implementation detail baked into `Nat`/`@word`'s own
    // constructor and mapping signatures instead. Told apart by the reserved `@`-name convention
    // system-generated declarations use, the same one `typing_info::sort_declaration_by_id` relies
    // on.
    let sorts: Vec<BasicSort> = spec
        .sort_declarations
        .iter()
        .filter(|d| d.expr.is_none() && !d.identifier.starts_with('@'))
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
            // Phase-3 already accepted this equation, so `None` means `Lowering`
            // is missing a construct it supports: an internal bug, and a hard
            // failure rather than a silently dropped rewrite rule.
            let lowered = lower_equation(ctx, spec, typing, eqn.condition.as_ref(), &eqn.lhs, &eqn.rhs, encoding)
                .unwrap_or_else(|| {
                    panic!("user equation '{eqn}' passed Phase-3 inference but failed Phase-4 lowering")
                });
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
                .unwrap_or_else(|| {
                    panic!("system equation '{eqn}' passed Phase-3 inference but failed Phase-4 lowering")
                });
            equations.push(DataEquation::new(&vars, lowered.condition, lowered.lhs, lowered.rhs));
        }
    }

    // Every container/function-update/comparison instantiation the
    // specification actually uses is monomorphized here, for this call only,
    // rather than during type-checking — see `docs/typecheck.md`'s
    // monomorphization-to-lowering milestone. `ctx` itself proved every
    // template's own equations exactly once, rigidly
    // (`check_container_templates`/`check_comparison_template`, run during
    // `from_untyped_with`); a scratch clone absorbs the work still needed
    // to turn that into ground content — checking a not-yet-seen
    // multi-argument function-update arity, interning a substituted sort —
    // without mutating the context the caller's `DataSpecification` still
    // holds.
    let mut scratch_ctx = ctx.clone();
    let mut scratch_sources = SourceMap::new();

    // Seeded empty, not with `basics`: `system`'s own constructors/mappings/
    // equations (basics and desugared structs) were already lowered above,
    // so seeding with a second copy here would duplicate them in the output.
    // `build_system_defined_specification`'s worklist only ever scans `spec`
    // to decide what to generate, never its own seed, so an empty seed
    // changes nothing about *which* instantiations it discovers.
    let (generated, instantiations) = build_system_defined_specification(
        &mut scratch_sources,
        spec,
        UntypedDataSpecification::default(),
        encoding,
    );
    let (mut generated, more_instantiations) =
        extend_system_with_inferred_sorts(&mut scratch_sources, &scratch_ctx, spec, &generated, encoding);
    let mut instantiations = instantiations;
    instantiations.extend(more_instantiations);

    resolve_data_specification_variables(&mut generated);

    // A cheap sanity net over the generated content (see
    // `check_system_specification`'s own doc comment) — checked against
    // `system`'s own declarations too (cloned in, not `generated` alone), so
    // a container equation referencing a basic-sort operator by name (e.g.
    // `+`) resolves correctly; `system` itself is left untouched; only
    // `generated`'s own content is ever lowered below, so this never
    // duplicates `system`'s content in the output. Should never fail for a
    // well-formed template: a failure here is a bug in the generator, not in
    // the user's specification (already fully checked before this call), so
    // it panics rather than threading a `Result` through lowering.
    let mut check_target = system.clone();
    check_target.merge(&generated);
    check_system_specification(spec, &check_target)
        .unwrap_or_else(|err| panic!("the generated system-defined specification is malformed: {err}"));

    assign_declaration_ids(&mut generated);

    // Every distinct arity a generated multi-argument function-update
    // instantiation uses gets its own generic template, checked once with its
    // type variable(s) held rigid, exactly like the six bundled container
    // templates and the comparison template — whose own results this scratch
    // context already inherited from `ctx`, since those are checked
    // unconditionally during `from_untyped_with` regardless of usage.
    let mut checked_arities = HashSet::new();
    for instantiation in &instantiations {
        if let Some(arity) = instantiation.template.strip_prefix("function_update_")
            && checked_arities.insert(arity.to_string())
        {
            let arity: usize = arity.parse().expect("`function_update_{arity}` names an integer arity");
            check_multi_argument_function_update_template(&mut scratch_ctx, arity).unwrap_or_else(|err| {
                panic!("the generated arity-{arity} function-update template failed its rigid check: {err}")
            });
        }
    }

    // Every equation is specialized from its own template's already-proven,
    // rigid typing by substitution, not re-inferred — see
    // `check_system_equations`/`specialize_template_typing`.
    check_system_equations(&mut scratch_ctx, spec, &generated, &instantiations)
        .unwrap_or_else(|err| panic!("a generated system equation failed to specialize: {err}"));

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
                .expect("check_system_equations resolved every generated equation's typing above")
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
            .unwrap_or_else(|| {
                panic!("generated equation '{eqn}' passed Phase-3 inference but failed Phase-4 lowering")
            });
            equations.push(DataEquation::new(&vars, lowered.condition, lowered.lhs, lowered.rhs));
        }
    }

    Mcrl2DataSpecification::new(sorts, aliases, constructors, mappings, equations)
}

#[cfg(test)]
mod tests {
    use merc_data::is_container_sort;
    use merc_data::is_data_binder;
    use merc_data::is_data_function_symbol;
    use merc_data::is_data_where_clause;
    use merc_data::is_function_sort;
    use merc_syntax::EqnSpecId;
    use merc_syntax::EquationId;
    use merc_syntax::Sort;
    use merc_syntax::UntypedDataSpecification;

    use super::DataExpression;
    use super::LoweredEquation;
    use super::NumberEncoding;
    use super::decimal_words_lsb_first;
    use super::lower_bool_literal;
    use super::lower_equation;
    use super::lower_number_literal;
    use super::lower_sort;
    use super::numeric_coerce;
    use crate::DataSpecification;

    fn typed(text: &str) -> DataSpecification {
        DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()).unwrap()
    }

    /// Lowers the single equation of `text`'s only `eqn` block (the shape
    /// every test spec here uses).
    fn lower(text: &str) -> Option<LoweredEquation> {
        let spec = typed(text);
        let eqn_spec = &spec.data_specification().equation_declarations[0];
        let eqn = &eqn_spec.equations[0];
        let typing = spec.equation_typing((EqnSpecId::new(0), EquationId::new(0)));
        lower_equation(
            spec.context(),
            spec.data_specification(),
            typing,
            eqn.condition.as_ref(),
            &eqn.lhs,
            &eqn.rhs,
            spec.number_encoding(),
        )
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_lower_primitive_sort() {
        let spec = typed("map f: Nat;");
        let sort = lower_sort(
            spec.context(),
            spec.data_specification(),
            spec.sort_of_map(merc_syntax::MapId::new(0)),
        );
        assert_eq!(sort.to_string(), "Nat");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_lower_generic_sort() {
        let spec = typed("map f: List(Nat);");
        let sort = lower_sort(
            spec.context(),
            spec.data_specification(),
            spec.sort_of_map(merc_syntax::MapId::new(0)),
        );
        assert!(is_container_sort(&sort));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_lower_function_sort() {
        let spec = typed("map f: Nat -> Bool;");
        let sort = lower_sort(
            spec.context(),
            spec.data_specification(),
            spec.sort_of_map(merc_syntax::MapId::new(0)),
        );
        assert!(is_function_sort(&sort));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_lower_def_sort() {
        let spec = typed("sort D; map f: D;");
        let sort = lower_sort(
            spec.context(),
            spec.data_specification(),
            spec.sort_of_map(merc_syntax::MapId::new(0)),
        );
        assert_eq!(sort.to_string(), "D");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_pos_literals() {
        assert_eq!(
            lower_number_literal("1", Sort::Pos, NumberEncoding::Binary).to_string(),
            "@c1"
        );
        assert_eq!(
            lower_number_literal("2", Sort::Pos, NumberEncoding::Binary).to_string(),
            "@cDub(false, @c1)"
        );
        assert_eq!(
            lower_number_literal("3", Sort::Pos, NumberEncoding::Binary).to_string(),
            "@cDub(true, @c1)"
        );
        assert_eq!(
            lower_number_literal("5", Sort::Pos, NumberEncoding::Binary).to_string(),
            "@cDub(true, @cDub(false, @c1))"
        );
        // 255 = 0b11111111 (all-ones): a `Pos` literal built from a decimal
        // string too large for a machine word exercises the
        // arbitrary-precision long-division encoding, not just a lookup.
        let text = lower_number_literal("255", Sort::Pos, NumberEncoding::Binary).to_string();
        assert_eq!(text.matches("@cDub(true, ").count(), 7, "{text}");
        assert!(text.contains("@c1)"), "{text}");
        assert_eq!(text.matches(')').count(), 7, "{text}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_nat_literals() {
        assert_eq!(
            lower_number_literal("0", Sort::Nat, NumberEncoding::Binary).to_string(),
            "@c0"
        );
        assert_eq!(
            lower_number_literal("2", Sort::Nat, NumberEncoding::Binary).to_string(),
            "@cNat(@cDub(false, @c1))"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_int_literal() {
        assert_eq!(
            lower_number_literal("0", Sort::Int, NumberEncoding::Binary).to_string(),
            "@cInt(@c0)"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_real_literal() {
        assert_eq!(
            lower_number_literal("0", Sort::Real, NumberEncoding::Binary).to_string(),
            "@cReal(@cInt(@c0), @c1)"
        );
        assert_eq!(
            lower_number_literal("1", Sort::Real, NumberEncoding::Binary).to_string(),
            "@cReal(@cInt(@cNat(@c1)), @c1)"
        );
    }

    /// `2^64` and `2^128` as decimal strings, the first values needing 2 and 3 digits.
    const TWO_POW_64: &str = "18446744073709551616";
    const TWO_POW_128: &str = "340282366920938463463374607431768211456";

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_decimal_words_lsb_first() {
        assert_eq!(decimal_words_lsb_first("0"), vec![0]);
        assert_eq!(decimal_words_lsb_first("1"), vec![1]);
        assert_eq!(decimal_words_lsb_first("18446744073709551615"), vec![u64::MAX]);
        // 2^64 is the first value that needs a second digit: 1 * 2^64 + 0.
        assert_eq!(decimal_words_lsb_first(TWO_POW_64), vec![0, 1]);
        assert_eq!(decimal_words_lsb_first("18446744073709551621"), vec![5, 1]);
        // 2^128 needs three digits.
        assert_eq!(decimal_words_lsb_first(TWO_POW_128), vec![0, 0, 1]);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_pos_literals_machine_word() {
        let e = NumberEncoding::MachineWord;
        assert_eq!(
            lower_number_literal("1", Sort::Pos, e).to_string(),
            "@most_significant_digit(1)"
        );
        assert_eq!(
            lower_number_literal("255", Sort::Pos, e).to_string(),
            "@most_significant_digit(255)"
        );
        // Two digits: 2^64 == 1 * 2^64 + 0.
        assert_eq!(
            lower_number_literal(TWO_POW_64, Sort::Pos, e).to_string(),
            "@concat_digit(@most_significant_digit(1), 0)"
        );
        // Three digits, most significant outermost-first.
        assert_eq!(
            lower_number_literal(TWO_POW_128, Sort::Pos, e).to_string(),
            "@concat_digit(@concat_digit(@most_significant_digit(1), 0), 0)"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_nat_literals_machine_word() {
        let e = NumberEncoding::MachineWord;
        // Zero is a single zero digit, not `@c0` as in the binary encoding.
        assert_eq!(
            lower_number_literal("0", Sort::Nat, e).to_string(),
            "@most_significant_digitNat(0)"
        );
        assert_eq!(
            lower_number_literal("42", Sort::Nat, e).to_string(),
            "@most_significant_digitNat(42)"
        );
        assert_eq!(
            lower_number_literal(TWO_POW_64, Sort::Nat, e).to_string(),
            "@concat_digit(@most_significant_digitNat(1), 0)"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_int_and_real_literals_machine_word() {
        let e = NumberEncoding::MachineWord;
        // `@cInt` / `@cReal` are shared with the binary encoding; only the
        // `Nat`/`Pos` payload changes.
        assert_eq!(
            lower_number_literal("0", Sort::Int, e).to_string(),
            "@cInt(@most_significant_digitNat(0))"
        );
        assert_eq!(
            lower_number_literal("7", Sort::Int, e).to_string(),
            "@cInt(@most_significant_digitNat(7))"
        );
        assert_eq!(
            lower_number_literal("1", Sort::Real, e).to_string(),
            "@cReal(@cInt(@most_significant_digitNat(1)), @most_significant_digit(1))"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_pos_to_nat_coercion_differs_per_encoding() {
        // The binary encoding embeds `Pos` into `Nat` with the `@cNat`
        // constructor; the machine-word `Nat` has no such constructor, so it
        // uses the `Pos2Nat` mapping instead.
        let pos: DataExpression = lower_number_literal("1", Sort::Pos, NumberEncoding::Binary);
        let widened = numeric_coerce(pos, Sort::Pos, Sort::Nat, NumberEncoding::Binary);
        assert_eq!(widened.to_string(), "@cNat(@c1)");

        let pos = lower_number_literal("1", Sort::Pos, NumberEncoding::MachineWord);
        let widened = numeric_coerce(pos, Sort::Pos, Sort::Nat, NumberEncoding::MachineWord);
        assert_eq!(widened.to_string(), "Pos2Nat(@most_significant_digit(1))");
    }

    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    #[test]
    fn test_bool_literals() {
        assert_eq!(lower_bool_literal(true).to_string(), "true");
        assert_eq!(lower_bool_literal(false).to_string(), "false");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_literal_sort_is_embedded() {
        // The `@cDub` `OpId` embeds its own (function) sort, `Bool # Pos -> Pos`.
        let cdub = lower_number_literal("2", Sort::Pos, NumberEncoding::Binary);
        assert!(is_function_sort(&cdub.data_function_symbol().sort()));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_user_op_application_no_coercion() {
        let equation = lower("map f: Bool -> Bool; var x: Bool; eqn f(x) = x;").expect("no coercion, no binder");
        assert_eq!(equation.lhs.to_string(), "f(x)");
        assert_eq!(equation.rhs.to_string(), "x");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_comparison_scheme_on_declared_sort() {
        let equation = lower("sort D; cons d: D; map b: Bool; eqn b = (d == d);").expect("== is a supported scheme");
        assert_eq!(equation.lhs.to_string(), "b");
        assert_eq!(equation.rhs.to_string(), "==(d, d)");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_if_scheme_on_declared_sort() {
        let equation = lower("sort D; cons d: D; map f: D; eqn f = if(true, d, d);").expect("if is a supported scheme");
        assert_eq!(equation.rhs.to_string(), "if(true, d, d)");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_literal_at_its_natural_sort() {
        // `1`'s minimal inferred sort is `Pos`, exactly `p`'s declared sort:
        // no coercion needed.
        let equation = lower("map p: Pos; eqn p = 1;").expect("no coercion needed");
        assert_eq!(equation.rhs.to_string(), "@c1");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_zero_literal_at_nat_sort() {
        let equation = lower("map n: Nat; eqn n = 0;").expect("0 is already Nat");
        assert_eq!(equation.rhs.to_string(), "@c0");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_equation_level_coercion_widens_rhs() {
        // `1`'s minimal sort is `Pos`, but `n` is declared `Nat`: the
        // equation itself needs a `Pos -> Nat` coercion, inserted on the
        // narrower (right-hand) side. The coercion is the constructor
        // application (`@cNat`) directly, not a call to a `Pos2Nat` conversion
        // function (that name is only a rewrite rule that reduces to this same
        // term, `nat.mcrl2`).
        let equation = lower("map n: Nat; eqn n = 1;").expect("Pos widens to Nat");
        assert_eq!(equation.rhs.to_string(), "@cNat(@c1)");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_equation_level_coercion_widens_lhs() {
        // Symmetric to the above, with the narrower side on the left.
        let equation = lower("map n: Nat; eqn 1 = n;").expect("Pos widens to Nat");
        assert_eq!(equation.lhs.to_string(), "@cNat(@c1)");
        assert_eq!(equation.rhs.to_string(), "n");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_direct_coercion_composes_intermediate_sorts() {
        // A `Pos -> Real` coercion composes every intermediate constructor
        // (`@cReal(@cInt(@cNat(x)), @c1)`), it does not call a single
        // `Pos2Real` function.
        let equation = lower("map r: Real; eqn r = 1;").expect("Pos widens to Real");
        assert_eq!(equation.rhs.to_string(), "@cReal(@cInt(@cNat(@c1)), @c1)");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_argument_coercion_widens_to_domain() {
        // `f`'s parameter is `Nat`, but `1` naturally infers to `Pos`: an
        // argument coercion.
        let equation = lower("map f: Nat -> Bool; eqn f(1) = true;").expect("Pos widens to Nat");
        assert_eq!(equation.lhs.to_string(), "f(@cNat(@c1))");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_fset_argument_widens_to_set() {
        // The `@set` constructor is inserted directly, not a call to
        // `@setfset` (a rewrite-system-only operator, per `set.mcrl2`'s own
        // comment that it "should not be part of the rewrite system").
        let equation =
            lower("map e: FSet(Nat); map s: Set(Nat) -> Bool; eqn s(e) = true;").expect("FSet widens to Set");
        assert_eq!(equation.lhs.to_string(), "s(@set(@false_, e))");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_fbag_argument_widens_to_bag() {
        let equation =
            lower("map e: FBag(Nat); map s: Bag(Nat) -> Bool; eqn s(e) = true;").expect("FBag widens to Bag");
        assert_eq!(equation.lhs.to_string(), "s(@bag(@zero_, e))");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_empty_list_lowers() {
        let equation = lower("map s: List(Nat); eqn s = [];").expect("empty list lowers");
        assert_eq!(equation.rhs.to_string(), "[]");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_empty_set_lowers() {
        let equation = lower("map s: FSet(Nat); eqn s = {};").expect("empty set lowers");
        assert_eq!(equation.rhs.to_string(), "{}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_empty_bag_lowers() {
        let equation = lower("map b: FBag(Nat); eqn b = {:};").expect("empty bag lowers");
        assert_eq!(equation.rhs.to_string(), "{:}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_fset_literal_lowers() {
        let equation = lower("map s: Nat -> FSet(Nat); var n: Nat; eqn s(n) = {n};").expect("singleton FSet lowers");
        // @fset_insert(n, {})
        assert!(equation.rhs.to_string().contains("@fset_insert"), "{}", equation.rhs);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_fset_literal_two_elements_lowers() {
        let equation = lower("map s: Nat # Nat -> FSet(Nat); var n: Nat; m: Nat; eqn s(n, m) = {n, m};")
            .expect("two-element FSet lowers");
        let rhs = equation.rhs.to_string();
        assert!(rhs.contains("@fset_insert"), "{rhs}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_fbag_literal_lowers() {
        let equation = lower("map b: Nat -> FBag(Nat); var n: Nat; eqn b(n) = {n: 1};").expect("singleton FBag lowers");
        // @fbag_cinsert(n, @cNat(@c1), {:})  — 1 infers Pos, widened to Nat
        let rhs = equation.rhs.to_string();
        assert!(rhs.contains("@fbag_cinsert"), "{rhs}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_empty_list_sort_is_embedded() {
        // The `[]` constant must carry a container (List) sort as its embedded sort.
        let equation = lower("map s: List(Nat); eqn s = [];").expect("empty list lowers");
        assert!(
            is_container_sort(&equation.rhs.data_sort()),
            "sort should be container: {}",
            equation.rhs.data_sort()
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_set_literal_widens_element_to_nat() {
        // `{1}` : FSet(Nat) — the `1` infers Pos, coerced to element sort Nat.
        let equation = lower("map s: FSet(Nat); eqn s = {1};").expect("FSet literal lowers");
        let rhs = equation.rhs.to_string();
        // The element is coerced Pos→Nat via @cNat.
        assert!(rhs.contains("@cNat"), "element coercion Pos→Nat expected in: {rhs}");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_lambda_lowers() {
        let equation = lower("map f: Bool -> Bool; eqn f = lambda x: Bool. x;").expect("lambda lowers");
        assert!(
            is_data_binder(&equation.rhs),
            "rhs should be a binder: {}",
            equation.rhs
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_forall_lowers() {
        let equation = lower("map b: Bool; eqn b = forall x: Bool. x;").expect("forall lowers");
        assert!(
            is_data_binder(&equation.rhs),
            "rhs should be a binder: {}",
            equation.rhs
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_exists_lowers() {
        let equation = lower("map b: Bool; eqn b = exists x: Bool. x;").expect("exists lowers");
        assert!(
            is_data_binder(&equation.rhs),
            "rhs should be a binder: {}",
            equation.rhs
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_setcomp_lowers() {
        // A comprehension becomes the `Set` constructor applied to its
        // characteristic function, not a `SetComp` binder — see
        // `Lowering::lower_setbagcomp`.
        let equation = lower("map s: Set(Nat); eqn s = { x: Nat | x == 0 };").expect("set comprehension lowers");
        assert_eq!(
            equation.rhs.to_string(),
            "@set(Binder(Lambda, [DataVarId(x, SortId(Nat))], DataAppl(OpIdNoIndex(==, SortArrow([SortId(Nat),SortId(Nat)], SortId(Bool))), DataVarId(x, SortId(Nat)), OpIdNoIndex(@c0, SortId(Nat)))), {})",
            "{}",
            equation.rhs
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_bagcomp_lowers() {
        // The body counts multiplicities, so it is widened to `Nat`; `x + 0`
        // is already `Nat` here, and the empty finite bag completes the pair.
        let equation = lower("map b: Bag(Nat); eqn b = { x: Nat | x + 0 };").expect("bag comprehension lowers");
        assert_eq!(
            equation.rhs.to_string(),
            "@bag(Binder(Lambda, [DataVarId(x, SortId(Nat))], DataAppl(OpIdNoIndex(+, SortArrow([SortId(Nat),SortId(Nat)], SortId(Nat))), DataVarId(x, SortId(Nat)), OpIdNoIndex(@c0, SortId(Nat)))), {:})",
            "{}",
            equation.rhs
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_bagcomp_widens_a_positive_body_to_nat() {
        // A `Pos` body cannot be the multiplicity function `@bag` requires, so
        // the literal is rebuilt at `Nat` (the same rule `coerce` applies to
        // any bare literal).
        let equation = lower("map b: Bag(Nat); eqn b = { x: Nat | 1 };").expect("bag comprehension lowers");
        assert_eq!(
            equation.rhs.to_string(),
            "@bag(Binder(Lambda, [DataVarId(x, SortId(Nat))], DataAppl(OpIdNoIndex(@cNat, SortArrow([SortId(Pos)], SortId(Nat))), OpIdNoIndex(@c1, SortId(Pos)))), {:})",
            "{}",
            equation.rhs
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_whr_lowers() {
        let equation = lower("map f: Bool; var x: Bool; eqn f = x whr x = true end;").expect("where clause lowers");
        assert!(
            is_data_where_clause(&equation.rhs),
            "rhs should be a where clause: {}",
            equation.rhs
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_lambda_variable_has_sort() {
        // The bound variable in the Binder must carry its declared sort.
        let equation = lower("map f: Bool -> Bool; eqn f = lambda x: Bool. x;").expect("lambda lowers");
        let rhs_str = equation.rhs.to_string();
        // The lowered term must contain a DataVarId encoding for x: Bool.
        assert!(
            rhs_str.contains("DataVarId"),
            "bound variable should be DataVarId in: {rhs_str}"
        );
        assert!(
            is_data_function_symbol(&equation.lhs),
            "lhs should be a function symbol: {}",
            equation.lhs
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_builtin_arithmetic_op() {
        // `+` is a system-declared op (`NameTarget::Op` after overload resolution against
        // the basic-sort system signature), but verifies that arithmetic resolves.
        let equation =
            lower("map n: Nat # Nat -> Nat; var a: Nat; b: Nat; eqn n(a, b) = a + b;").expect("arithmetic lowers");
        assert_eq!(equation.rhs.to_string(), "+(a, b)");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_builtin_polymorphic_container_op() {
        // `in` is a POLYMORPHIC_SIGNATURE op (`NameTarget::Builtin`) whose
        // inferred sort is the concrete instantiation; the lowered term embeds
        // that sort directly.
        let equation = lower("map b: Nat # Set(Nat) -> Bool; var n: Nat; s: Set(Nat); eqn b(n, s) = n in s;")
            .expect("container op lowers with step 3 fix");
        assert_eq!(equation.rhs.to_string(), "in(n, s)");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_builtin_func_update() {
        // `@func_update` is lowered by lower.rs to an Application; with the
        // step-3 fix its Builtin target uses the inferred sort directly.
        let equation = lower("map f: Nat -> Bool; map g: Nat -> Nat -> Bool; var n: Nat; eqn g(n) = f[n -> true];")
            .expect("@func_update lowers with step 3 fix");
        assert_eq!(equation.rhs.to_string(), "@func_update(f, n, true)");
    }
}
