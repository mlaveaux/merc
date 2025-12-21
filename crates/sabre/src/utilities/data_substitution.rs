#![forbid(unsafe_code)]

use merc_aterm::Protected;
use merc_aterm::Term;
use merc_aterm::storage::ThreadTermPool;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_data::is_data_application;

use super::DataPosition;

pub type DataSubstitutionBuilder = Protected<Vec<DataExpressionRef<'static>>>;

/// This function substitutes the term 't' at the position 'p' with 'new_subterm', see [super::substitute].
pub fn data_substitute(
    tp: &ThreadTermPool,
    t: &DataExpressionRef<'_>,
    new_subterm: DataExpression,
    position: &DataPosition,
) -> DataExpression {
    let mut args = Protected::new(vec![]);
    substitute_rec(tp, t, new_subterm, position.indices(), &mut args, 0)
}

/// This is the same as [data_substitute], but it uses a [DataSubstitutionBuilder] to store the arguments temporarily.
pub fn data_substitute_with(
    builder: &mut DataSubstitutionBuilder,
    tp: &ThreadTermPool,
    t: &DataExpressionRef<'_>,
    new_subterm: DataExpression,
    position: &DataPosition,
) -> DataExpression {
    substitute_rec(tp, t, new_subterm, position.indices(), builder, 0)
}

/// The recursive implementation for [data_substitute]
///
/// 'depth'         -   Used to keep track of the depth in 't'. Function should be called with
///                     'depth' = 0.
fn substitute_rec(
    tp: &ThreadTermPool,
    t: &DataExpressionRef<'_>,
    new_subterm: DataExpression,
    p: &[usize],
    args: &mut DataSubstitutionBuilder,
    depth: usize,
) -> DataExpression {
    if p.len() == depth {
        // in this case we have arrived at the place where 'new_subterm' needs to be injected
        new_subterm
    } else {
        // else recurse deeper into 't', do not subtract 1 from the index, since we are using DataPosition
        let new_child_index = p[depth];
        let new_child = substitute_rec(tp, &t.arg(new_child_index).into(), new_subterm, p, args, depth + 1);

        debug_assert!(
            is_data_application(t),
            "Can only perform data substitution on DataApplications"
        );

        let mut write_args = args.write();
        for (index, arg) in t.arguments().enumerate() {
            if index == new_child_index {
                let t = write_args.protect(&new_child);
                write_args.push(t.into());
            } else {
                let t = write_args.protect(&arg);
                write_args.push(t.into());
            }
        }

        // Avoid the (more expensive) DataApplication constructor by simply having the data_function_symbol in args.
        let result = tp.create_term(&t.get_head_symbol(), &write_args);
        drop(write_args);

        // TODO: When write is dropped we check whether all terms where inserted, but this clear violates that assumption.
        args.write().clear();
        result.protect().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use merc_aterm::storage::THREAD_TERM_POOL;

    use crate::utilities::DataPosition;
    use crate::utilities::DataPositionIndexed;

    #[test]
    fn test_data_substitute() {
        let t = DataExpression::from_string("s(s(a))").unwrap();
        let t0 = DataExpression::from_string("0").unwrap();

        // substitute the a for 0 in the term s(s(a))
        let result =
            THREAD_TERM_POOL.with_borrow(|tp| data_substitute(tp, &t.copy(), t0.clone(), &DataPosition::new(&[1, 1])));

        // Check that indeed the new term as a 0 at position 1.1.
        assert_eq!(t0, result.get_data_position(&DataPosition::new(&vec![1, 1])).protect());
    }
}
