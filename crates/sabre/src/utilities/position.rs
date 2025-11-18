//! Module for storing positions of terms
#![forbid(unsafe_code)]

use core::fmt;
use std::collections::VecDeque;

use merc_aterm::ATermRef;

use merc_aterm::Term;
use smallvec::SmallVec;
use smallvec::smallvec;

/// An ExplicitPosition stores a list of position indices. The index starts at 1.
/// The subterm of term s(s(0)) at position 1.1 is 0.
/// The empty position, aka the root term, is represented by the symbol ε.
/// Indices are stored in a SmallVec, which is configured to store 4 elements.
/// If the position contains a maximum of 4 elements it is stored on the stack.
/// If the position is longer a heap allocation is made.
#[repr(transparent)]
#[derive(Hash, Clone, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct ExplicitPosition {
    indices: SmallVec<[usize; 4]>,
}

impl ExplicitPosition {
    /// Creates a new ExplicitPosition from a slice of indices.
    pub fn new(indices: &[usize]) -> Self {
        Self {
            indices: SmallVec::from(indices),
        }
    }

    /// Create the empty position.
    pub fn empty() -> Self {
        Self { indices: smallvec![] }
    }

    /// Returns the length of the position indices
    pub fn len(&self) -> usize {
        self.indices.len()
    }

    /// Returns the indices of the position.
    pub fn indices(&self) -> &[usize] {
        &self.indices
    }

    /// Returns true iff the position is empty.
    pub fn is_empty(&self) -> bool {
        self.indices.len() == 0
    }

    /// Add the index to the position.
    pub fn push(&mut self, index: usize) {
        self.indices.push(index);
    }
}

impl<'a, 'b, T: Term<'a, 'b>> PositionIndexed<'b> for T
where
    'a: 'b,
{
    type Target<'c>
        = ATermRef<'c>
    where
        Self: 'c,
        Self: 'b;

    fn get_position(&'b self, position: &ExplicitPosition) -> Self::Target<'b> {
        let mut result = self.copy();

        for index in &position.indices {
            result = result.arg(index - 1); // Note that positions are 1 indexed.
        }

        result
    }
}

pub trait PositionIndexed<'b> {
    type Target<'a>
    where
        Self: 'a,
        Self: 'b;

    /// Returns the Target at the given position.
    fn get_position(&'b self, position: &ExplicitPosition) -> Self::Target<'b>;
}

impl fmt::Display for ExplicitPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl fmt::Debug for ExplicitPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.indices.is_empty() {
            write!(f, "ε")?;
        } else {
            let mut first = true;
            for p in &self.indices {
                if first {
                    write!(f, "{p}")?;
                    first = false;
                } else {
                    write!(f, ".{p}")?;
                }
            }
        }

        Ok(())
    }
}

/// An iterator over all (term, position) pairs of the given ATerm.
pub struct PositionIterator<'a> {
    queue: VecDeque<(ATermRef<'a>, ExplicitPosition)>,
}

impl<'a> PositionIterator<'a> {
    pub fn new(t: ATermRef<'a>) -> PositionIterator<'a> {
        PositionIterator {
            queue: VecDeque::from([(t, ExplicitPosition::empty())]),
        }
    }
}

impl<'a> Iterator for PositionIterator<'a> {
    type Item = (ATermRef<'a>, ExplicitPosition);

    fn next(&mut self) -> Option<Self::Item> {
        if self.queue.is_empty() {
            None
        } else {
            // Get a subterm to inspect
            let (term, pos) = self.queue.pop_front().unwrap();

            // Put subterms in the queue
            for (i, argument) in term.arguments().enumerate() {
                let mut new_position = pos.clone();
                new_position.indices.push(i + 1);
                self.queue.push_back((argument, new_position));
            }

            Some((term, pos))
        }
    }
}

#[cfg(test)]
mod tests {
    use merc_aterm::ATerm;

    use super::*;

    #[test]
    fn test_get_position() {
        let t = ATerm::from_string("f(g(a),b)").unwrap();
        let expected = ATerm::from_string("a").unwrap();

        assert_eq!(t.get_position(&ExplicitPosition::new(&[1, 1])), expected.copy());
    }

    #[test]
    fn test_position_iterator() {
        let t = ATerm::from_string("f(g(a),b)").unwrap();

        for (term, pos) in PositionIterator::new(t.copy()) {
            assert_eq!(
                t.get_position(&pos),
                term,
                "The resulting (subterm, position) pair doesn't match the get_position implementation"
            );
        }

        assert_eq!(
            PositionIterator::new(t.copy()).count(),
            4,
            "The number of subterms doesn't match the expected value"
        );
    }
}
