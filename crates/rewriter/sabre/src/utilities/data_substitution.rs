use merc_aterm::Protected;
use merc_aterm::Term;
use merc_aterm::storage::ThreadTermPool;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_data::is_data_application;

use super::DataPosition;

pub type DataSubstitutionBuilder = Protected<Vec<DataExpressionRef<'static>>>;

/// Creates a new term with the subterm at `position` replaced by `new_subterm`.
///
/// The term is rebuilt bottom-up: e.g. replacing `a` with `0` in `s(s(a))` at position `[1, 1]`
/// first constructs `s(0)`, then `s(s(0))`.
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
/// Uses `depth` to keep track of the depth in 't', initially 0.
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

        let result = args.write(|write_args| {
            for (index, arg) in t.arguments().enumerate() {
                if index == new_child_index {
                    // Safety: t is pushed into the container on the next line.
                    let t = unsafe { write_args.protect(&new_child) };
                    write_args.push(t.into());
                } else {
                    // Safety: t is pushed into the container on the next line.
                    let t = unsafe { write_args.protect(&arg) };
                    write_args.push(t.into());
                }
            }

            // Avoid the (more expensive) DataApplication constructor by simply having the data_function_symbol in args.
            tp.create_term(&t.get_head_symbol(), write_args).protect().into()
        });

        // Clear the args buffer for reuse.
        args.write(|write_args| write_args.clear());
        result
    }
}
