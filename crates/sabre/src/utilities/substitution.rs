#![forbid(unsafe_code)]

use merc_aterm::ATerm;
use merc_aterm::ATermRef;
use merc_aterm::Protected;
use merc_aterm::Term;
use merc_aterm::storage::ThreadTermPool;

pub type SubstitutionBuilder = Protected<Vec<ATermRef<'static>>>;

/// Creates a new term where a subterm is replaced with another term.
///
/// # Parameters
/// 't'             -   The original term
/// 'new_subterm'   -   The subterm that will be injected
/// 'p'             -   The place in 't' on which 'new_subterm' will be placed,
///                     given as a slice of position indexes
///
/// # Example
///
/// The term is constructed bottom up. As an example take the term s(s(a)).
/// Lets say we want to replace the a with the term 0. Then we traverse the term
/// until we have arrived at a and replace it with 0. We then construct s(0)
/// and then construct s(s(0)).
pub fn substitute<'a, 'b>(tp: &ThreadTermPool, t: &'b impl Term<'a, 'b>, new_subterm: ATerm, p: &[usize]) -> ATerm {
    let mut args = Protected::new(vec![]);
    substitute_rec(tp, t, new_subterm, p, &mut args, 0)
}

pub fn substitute_with<'a, 'b>(
    builder: &mut SubstitutionBuilder,
    tp: &ThreadTermPool,
    t: &'b impl Term<'a, 'b>,
    new_subterm: ATerm,
    p: &[usize],
) -> ATerm {
    substitute_rec(tp, t, new_subterm, p, builder, 0)
}

/// The recursive implementation for subsitute
///
/// 'depth'         -   Used to keep track of the depth in 't'. Function should be called with
///                     'depth' = 0.
fn substitute_rec<'a, 'b>(
    tp: &ThreadTermPool,
    t: &'b impl Term<'a, 'b>,
    new_subterm: ATerm,
    p: &[usize],
    args: &mut SubstitutionBuilder,
    depth: usize,
) -> ATerm {
    if p.len() == depth {
        // in this case we have arrived at the place where 'new_subterm' needs to be injected
        new_subterm
    } else {
        // else recurse deeper into 't'
        let new_child_index = p[depth] - 1;
        let new_child = substitute_rec(tp, &t.arg(new_child_index), new_subterm, p, args, depth + 1);

        let mut write_args = args.write();
        for (index, arg) in t.arguments().enumerate() {
            if index == new_child_index {
                let t = write_args.protect(&new_child);
                write_args.push(t);
            } else {
                let t = write_args.protect(&arg);
                write_args.push(t);
            }
        }

        let result = tp.create_term(&t.get_head_symbol(), &write_args);
        drop(write_args);

        // TODO: When write is dropped we check whether all terms where inserted, but this clear violates that assumption.
        args.write().clear();
        result.protect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use merc_aterm::storage::THREAD_TERM_POOL;

    use crate::utilities::ExplicitPosition;
    use crate::utilities::PositionIndexed;

    #[test]
    fn test_substitute() {
        let t = ATerm::from_string("s(s(a))").unwrap();
        let t0 = ATerm::from_string("0").unwrap();

        // substitute the a for 0 in the term s(s(a))
        let result = THREAD_TERM_POOL.with_borrow(|tp| substitute(tp, &t, t0.clone(), &vec![1, 1]));

        // Check that indeed the new term as a 0 at position 1.1.
        assert_eq!(t0, result.get_position(&ExplicitPosition::new(&vec![1, 1])).protect());
    }
}
