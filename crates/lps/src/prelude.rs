use ahash::HashSet;
use ahash::HashSetExt;
use itertools::Itertools;
use merc_aterm::Term;
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
use merc_typecheck::NumberEncoding;

/// Merges `spec` (an `.lps` file's `user_defined_*` sections, per mCRL2's own
/// naming: only the sorts/constructors/mappings/equations the source
/// specification itself declared) with the standard-library prelude
/// (`Bool`, `Nat`, `List`, machine words, …), which the `.lps` format omits
/// entirely since every mCRL2 toolset already has it built in.
///
/// Without this, rewriting anything that touches a built-in sort — even
/// `n < 3` — gets stuck: a `.lps` file for a specification that declares no
/// custom data at all carries zero equations by itself (confirmed against a
/// real `mcrl22lps`-produced fixture; see `tests/explore_lps_tests.rs`).
///
/// Container sorts (`List(D)`, `Set(D)`, `Bag(D)`) are only instantiated by
/// `merc_typecheck` for element sorts `D` actually mentioned somewhere in the
/// typechecked source text — never unconditionally, since the concrete
/// element sort determines the constructor/mapping/equation set. An empty
/// prelude source would therefore never instantiate any container `spec`
/// itself relies on, so this derives trigger declarations from `spec`'s own
/// constructor/mapping/equation sorts (see [`trigger_declarations_for`])
/// instead of parsing a fixed empty string.
pub(crate) fn with_standard_prelude(spec: &Mcrl2DataSpecification) -> Mcrl2DataSpecification {
    let source = trigger_declarations_for(spec);
    let untyped = UntypedDataSpecification::parse(&source)
        .unwrap_or_else(|err| panic!("the synthesised trigger specification must parse: {err}\n{source}"));
    // mCRL2's own toolset always represents Pos/Nat/Int/Real as 64-bit
    // machine-word digit chains (`@most_significant_digitNat`, …) — never the
    // recursive binary encoding, which is `merc_typecheck`'s own default.
    // Every `.lps` file this crate reads was produced by that toolset, so the
    // merged prelude must match it or equations for the digit-chain
    // constructors the file actually uses simply won't exist.
    let typed = DataSpecification::from_untyped_with(untyped, NumberEncoding::MachineWord)
        .unwrap_or_else(|err| panic!("the synthesised trigger specification must typecheck: {err}\n{source}"));
    let prelude = typed.lower_data_specification();

    let mut sorts = prelude.sorts().to_vec();
    sorts.extend(spec.sorts().iter().cloned());

    let mut aliases = prelude.aliases().to_vec();
    aliases.extend(spec.aliases().iter().cloned());

    let mut constructors = prelude.constructors().to_vec();
    constructors.extend(spec.constructors().iter().cloned());

    let mut mappings = prelude.mappings().to_vec();
    mappings.extend(spec.mappings().iter().cloned());

    let mut equations = prelude.equations().to_vec();
    equations.extend(spec.equations().iter().cloned());

    Mcrl2DataSpecification::new(sorts, aliases, constructors, mappings, equations)
}

/// Synthesises mCRL2 source text declaring one dummy map per container sort
/// (`List(D)`/`Set(D)`/`Bag(D)`) reachable from `spec`'s own constructors,
/// mappings, and equations: typechecking such a declaration is what makes
/// `merc_typecheck` instantiate that container's own operations.
///
/// Only sorts `spec` itself references are covered. A container sort that
/// appears solely as a process parameter's sort, with no user-declared
/// map/eqn touching it, is a known residual gap.
fn trigger_declarations_for(spec: &Mcrl2DataSpecification) -> String {
    let mut seen = HashSet::new();
    let mut containers = Vec::new();

    for constructor in spec.constructors() {
        collect_trigger_sorts(constructor.sort(), &mut seen, &mut containers);
    }
    for mapping in spec.mappings() {
        collect_trigger_sorts(mapping.sort(), &mut seen, &mut containers);
    }
    for equation in spec.equations() {
        for variable in equation.variables().iter() {
            collect_trigger_sorts(variable.sort(), &mut seen, &mut containers);
        }
        if let Some(condition) = equation.condition() {
            collect_sorts_in_expression(condition, &mut seen, &mut containers);
        }
        collect_sorts_in_expression(equation.lhs(), &mut seen, &mut containers);
        collect_sorts_in_expression(equation.rhs(), &mut seen, &mut containers);
    }

    containers
        .iter()
        .enumerate()
        .map(|(i, sort)| format!("map __trigger_{i}: {sort} -> {sort};"))
        .join("\n")
}

/// Recursively records every container sort (`List`/`Set`/`Bag`/`FSet`/`FBag`
/// — mCRL2's grammar accepts all five directly, e.g. a generated `.lps` may
/// declare a process parameter of sort `FSet(D)` without ever mentioning
/// `Set(D)`) reachable from `sort`, descending into function domains/codomains
/// and container element sorts. `seen` deduplicates by sort so a shared
/// substructure (e.g. `Nat` appearing everywhere) is only walked once.
fn collect_trigger_sorts(
    sort: SortExpressionRef<'_>,
    seen: &mut HashSet<SortExpression>,
    out: &mut Vec<SortExpression>,
) {
    let owned = sort.protect();
    if !seen.insert(owned.clone()) {
        return;
    }

    if is_container_sort(&sort) {
        let cons: SortConsRef = Term::copy(&sort).into();
        out.push(owned);
        collect_trigger_sorts(cons.element_sort(), seen, out);
    } else if is_function_sort(&sort) {
        let arrow: SortArrowRef = Term::copy(&sort).into();
        for domain in arrow.domain().iter() {
            collect_trigger_sorts(domain.copy(), seen, out);
        }
        collect_trigger_sorts(arrow.codomain(), seen, out);
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
