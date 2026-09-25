use rand::Rng;
use rand::RngExt;
use rand::seq::IndexedRandom;

use merc_utilities::Span;

use crate::ComplexSort;
use crate::SortDecl;
use crate::SortExpression;
use crate::SortExpressionKind;
use crate::UntypedDataSpecification;
use crate::respan;
use crate::syntax_tree::ConstructorDecl;
use crate::syntax_tree::Sort;

const BASIC_SORTS: &[Sort] = &[Sort::Bool, Sort::Pos, Sort::Nat, Sort::Int, Sort::Real];
const CONTAINERS: &[ComplexSort] = &[
    ComplexSort::List,
    ComplexSort::Set,
    ComplexSort::FSet,
    ComplexSort::FBag,
    ComplexSort::Bag,
];

/// Generates `sort_count` random sort declarations, each referencing only sorts declared
/// earlier in the same list (plus the built-in basic sorts) -- so the dependency graph is
/// acyclic by construction and every generated sort is finite/constructible, with no need to
/// separately check mCRL2's "recursive struct needs a base case" well-formedness rule. Every
/// declaration is a `sort Name = <expr>;` alias (never a bare, unconstructible `sort Name;`),
/// one of: an alias to a basic sort, a struct with 1-3 constructors, a container
/// (`List`/`Set`/`FSet`/`FBag`/`Bag`) of an earlier sort, or a function sort over earlier sorts.
pub fn random_data_specification<R: Rng>(rng: &mut R, sort_count: usize, max_depth: usize) -> UntypedDataSpecification {
    let mut sort_names: Vec<String> = Vec::with_capacity(sort_count);
    let mut sort_declarations = Vec::with_capacity(sort_count);
    let mut constructor_id = 0usize;

    for i in 0..sort_count {
        let name = format!("Sort{i}");
        let expr = random_sort_expr(rng, &sort_names, max_depth, &mut constructor_id);
        sort_declarations.push(SortDecl::new(name.clone(), Some(expr), Span::default()));
        sort_names.push(name);
    }

    UntypedDataSpecification {
        sort_declarations,
        constructor_declarations: Vec::new(),
        map_declarations: Vec::new(),
        equation_declarations: Vec::new(),
        type_var_declarations: Vec::new(),
    }
}

/// Picks a leaf sort: a basic sort, or (once at least one exists) an earlier declared sort.
fn random_leaf_sort<R: Rng>(rng: &mut R, earlier: &[String]) -> SortExpression {
    let use_earlier = !earlier.is_empty() && rng.random_bool(0.5);
    if use_earlier {
        SortExpressionKind::Reference(earlier.choose(rng).unwrap().clone()).into()
    } else {
        SortExpressionKind::Simple(*BASIC_SORTS.choose(rng).unwrap()).into()
    }
}

/// Generates one sort's defining expression. `depth` bounds how many further constructor
/// levels (struct field / container element / function domain-range) may nest before falling
/// back to a leaf sort.
fn random_sort_expr<R: Rng>(
    rng: &mut R,
    earlier: &[String],
    depth: usize,
    constructor_id: &mut usize,
) -> SortExpression {
    if depth == 0 {
        return random_leaf_sort(rng, earlier);
    }

    match rng.random_range(0..4u8) {
        0 => random_leaf_sort(rng, earlier),
        1 => {
            let constructor_count = rng.random_range(1..=3usize);
            let inner = (0..constructor_count)
                .map(|_| random_constructor(rng, earlier, depth - 1, constructor_id))
                .collect();
            SortExpressionKind::Struct { inner }.into()
        }
        2 => {
            let container = *CONTAINERS.choose(rng).unwrap();
            let element = random_sort_expr(rng, earlier, depth - 1, constructor_id);
            SortExpressionKind::Complex(container, Box::new(element)).into()
        }
        3 => {
            let domain = random_leaf_sort(rng, earlier);
            let range = random_sort_expr(rng, earlier, depth - 1, constructor_id);
            SortExpressionKind::Function {
                domain: Box::new(domain),
                range: Box::new(range),
            }
            .into()
        }
        _ => unreachable!(),
    }
}

fn random_constructor<R: Rng>(
    rng: &mut R,
    earlier: &[String],
    depth: usize,
    constructor_id: &mut usize,
) -> ConstructorDecl {
    let name = format!("c{constructor_id}");
    *constructor_id += 1;

    let field_count = rng.random_range(0..=2usize);
    let args = (0..field_count)
        .map(|field_index| {
            let field_name = format!("{name}_f{field_index}");
            (
                Some(respan(Span::default(), field_name)),
                random_sort_expr(rng, earlier, depth, constructor_id),
            )
        })
        .collect::<Vec<_>>();

    ConstructorDecl {
        name: respan(Span::default(), name),
        args,
        recogniser: None,
    }
}
