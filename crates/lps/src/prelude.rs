use ahash::HashSet;
use ahash::HashSetExt;
use itertools::Itertools;
use merc_aterm::Term;
use merc_data::BasicSort;
use merc_data::DataExpressionRef;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::Mcrl2DataSpecification;
use merc_data::SortArrowRef;
use merc_data::SortConsRef;
use merc_data::SortExpression;
use merc_data::SortExpressionRef;
use merc_data::is_container_sort;
use merc_data::is_data_binder;
use merc_data::is_data_machine_number;
use merc_data::is_data_variable;
use merc_data::is_data_where_clause;
use merc_data::is_function_sort;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;

/// Merges `spec` (an `.lps` file's `user_defined_*` sections, per mCRL2's own
/// naming: only the sorts/constructors/mappings/equations the source
/// specification itself declared) with the standard-library prelude
/// (`Bool`, `Nat`, `List`, machine words, …), which the `.lps` format omits
/// entirely since every mCRL2 toolset already has it built in.
pub(crate) fn with_standard_prelude(spec: &Mcrl2DataSpecification) -> Mcrl2DataSpecification {
    let source = trigger_declarations_for(spec);
    let untyped = UntypedDataSpecification::parse(&source)
        .unwrap_or_else(|err| panic!("the synthesised trigger specification must parse: {err}\n{source}"));

    let typed = DataSpecification::from_untyped(untyped)
        .unwrap_or_else(|err| panic!("the synthesised trigger specification must typecheck: {err}\n{source}"));
    let prelude = typed.lower_data_specification();

    prelude.merge(spec)
}

/// Synthesises mCRL2 source text declaring one dummy map per sort reachable
/// from `spec`'s own constructors, mappings, and equations (plus `Bool`,
/// always — see below): typechecking such a declaration is what makes
/// `merc_typecheck` instantiate that sort's container operations
/// (`List(D)`/`Set(D)`/`Bag(D)`, …) when it is a container, and its
/// polymorphic comparison scheme (`==`, `<`, `if`, …) regardless.
///
/// `Bool` is triggered unconditionally, even when nothing below would
/// otherwise reach it: a summand condition's `if`-compiled disjunctions need
/// `if`'s own defining equations for `Bool`, but summand conditions live in
/// the LPS's process section, which `crate::io::read_lps` only reads *after*
/// this prelude is built — so they can never be seen here.
///
/// Beyond that, only sorts `spec` itself references are covered. A sort that
/// appears solely as a process parameter's sort, with no user-declared
/// map/eqn touching it, is a known residual gap.
fn trigger_declarations_for(spec: &Mcrl2DataSpecification) -> String {
    let mut seen = HashSet::new();
    let mut sorts = Vec::new();

    let bool_sort: SortExpression = BasicSort::new("Bool").into();
    collect_trigger_sorts(bool_sort.copy(), &mut seen, &mut sorts);

    for constructor in spec.constructors() {
        collect_trigger_sorts(constructor.sort(), &mut seen, &mut sorts);
    }
    for mapping in spec.mappings() {
        collect_trigger_sorts(mapping.sort(), &mut seen, &mut sorts);
    }
    for equation in spec.equations() {
        for variable in equation.variables().iter() {
            collect_trigger_sorts(variable.sort(), &mut seen, &mut sorts);
        }
        if let Some(condition) = equation.condition() {
            collect_sorts_in_expression(condition, &mut seen, &mut sorts);
        }
        collect_sorts_in_expression(equation.lhs(), &mut seen, &mut sorts);
        collect_sorts_in_expression(equation.rhs(), &mut seen, &mut sorts);
    }

    sorts
        .iter()
        .enumerate()
        .map(|(i, sort)| format!("map __trigger_{i}: {sort} -> {sort};"))
        .join("\n")
}

/// Recursively records every sort reachable from `sort` that the trigger
/// mechanism needs a dummy `map` declaration for: every container sort
/// (`List`/`Set`/`Bag`/`FSet`/`FBag` — mCRL2's grammar accepts all five
/// directly, e.g. a generated `.lps` may declare a process parameter of sort
/// `FSet(D)` without ever mentioning `Set(D)`), whose element sort is
/// recursed into too, and every other concrete sort (`Bool`, a user-declared
/// struct, …), so its comparison scheme gets instantiated alongside it.
/// Descends into function domains/codomains without ever triggering a
/// function sort itself — `map __trigger_i: (A -> B) -> (A -> B);` is not a
/// meaningful comparison-scheme target. `seen` deduplicates by sort so a
/// shared substructure (e.g. `Nat` appearing everywhere) is only walked once.
fn collect_trigger_sorts(
    sort: SortExpressionRef<'_>,
    seen: &mut HashSet<SortExpression>,
    out: &mut Vec<SortExpression>,
) {
    let owned = sort.protect();
    if !seen.insert(owned.clone()) {
        return;
    }

    if is_function_sort(&sort) {
        let arrow: SortArrowRef = Term::copy(&sort).into();
        for domain in arrow.domain().iter() {
            collect_trigger_sorts(domain.copy(), seen, out);
        }
        collect_trigger_sorts(arrow.codomain(), seen, out);
        return;
    }

    out.push(owned);
    if is_container_sort(&sort) {
        let cons: SortConsRef = Term::copy(&sort).into();
        collect_trigger_sorts(cons.element_sort(), seen, out);
    }
}

/// Recursively records every container sort reachable from any function
/// symbol or variable occurring in `expr`, the expression-tree counterpart of
/// [`collect_trigger_sorts`].
///
/// Does not descend into binders or where clauses, matching the same
/// limitation `crate::explore`'s free-variable analysis documents: neither is
/// expected in an `.lps`'s equations today.
fn collect_sorts_in_expression(
    expr: DataExpressionRef<'_>,
    seen: &mut HashSet<SortExpression>,
    out: &mut Vec<SortExpression>,
) {
    if is_data_variable(&expr) {
        let variable: DataVariable = expr.protect().into();
        collect_trigger_sorts(variable.sort(), seen, out);
        return;
    }
    if is_data_machine_number(&expr) || is_data_binder(&expr) || is_data_where_clause(&expr) {
        return;
    }

    if let Some(symbol) = expr.try_data_function_symbol() {
        let symbol: DataFunctionSymbol = symbol.protect();
        collect_trigger_sorts(symbol.sort(), seen, out);
    }
    for arg in expr.data_arguments() {
        collect_sorts_in_expression(arg, seen, out);
    }
}
