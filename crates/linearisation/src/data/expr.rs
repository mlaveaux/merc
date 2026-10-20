use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::SortArrow;
use merc_data::SortExpression;
use merc_data::sort_of;
use merc_utilities::BooleanTerm;
use merc_utilities::optimized_conjunction;

/// `lhs == rhs`, which is `true` when both sides are the same term.
pub(crate) fn equal(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
    if lhs == rhs {
        return DataExpression::true_();
    }
    let sort = sort_of(&lhs);
    let bool_sort = DataExpression::true_().data_sort();
    let arrow: SortExpression = SortArrow::new(&[sort.clone(), sort], bool_sort).into();
    let head = DataFunctionSymbol::with_sort("==", arrow.copy());
    DataApplication::with_args(&head, &[lhs, rhs]).into()
}

/// The conjunction of the pairwise equalities of two argument vectors of the same length.
pub(crate) fn equal_vectors(lhs: &[DataExpression], rhs: &[DataExpression]) -> DataExpression {
    debug_assert_eq!(lhs.len(), rhs.len());
    optimized_conjunction(lhs.iter().zip(rhs).map(|(l, r)| equal(l.clone(), r.clone())))
}
