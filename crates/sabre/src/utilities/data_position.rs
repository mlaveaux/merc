#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::fmt;

use merc_data::DataExpression;
use merc_data::DataExpressionRef;

use super::ExplicitPosition;

/// A newtype wrapper around [ExplicitPosition] specifically for data expressions
/// This provides type safety and clarity when dealing with positions in data expressions
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataPosition(ExplicitPosition);

impl DataPosition {
    /// Creates a new empty position
    pub fn empty() -> Self {
        Self(ExplicitPosition::empty())
    }

    /// Creates a new position from a slice of indices
    pub fn new(indices: &[usize]) -> Self {
        Self(ExplicitPosition::new(indices))
    }

    /// Returns the underlying indices
    pub fn indices(&self) -> &[usize] {
        self.0.indices()
    }

    /// Returns the length of the position indices
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if the position is empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Adds the index to the position
    pub fn push(&mut self, index: usize) {
        self.0.push(index);
    }
}

impl fmt::Display for DataPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for DataPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A specialisation of the [super::PositionIndexed] trait for [DataExpression]. This is used to keep the indexing consistent.
pub trait DataPositionIndexed<'b> {
    type Target<'a>
    where
        Self: 'a,
        Self: 'b;

    /// Returns the Target at the given position.
    fn get_data_position(&'b self, position: &DataPosition) -> Self::Target<'b>;
}

impl<'b> DataPositionIndexed<'b> for DataExpression {
    type Target<'a>
        = DataExpressionRef<'a>
    where
        Self: 'a;

    fn get_data_position(&'b self, position: &DataPosition) -> Self::Target<'b> {
        let mut result = self.copy();

        for index in position.indices() {
            result = result.data_arg(*index - 1); // Note that positions are 1 indexed.
        }

        result
    }
}

impl<'b> DataPositionIndexed<'b> for DataExpressionRef<'b> {
    type Target<'a>
        = DataExpressionRef<'a>
    where
        Self: 'a;

    fn get_data_position(&'b self, position: &DataPosition) -> Self::Target<'b> {
        let mut result = self.copy();

        for index in position.indices() {
            result = result.data_arg(*index - 1); // Note that positions are 1 indexed.
        }

        result
    }
}

/// An iterator over all (term, position) pairs of the given [DataExpression].
pub struct DataPositionIterator<'a> {
    queue: VecDeque<(DataExpressionRef<'a>, DataPosition)>,
}

impl<'a> DataPositionIterator<'a> {
    pub fn new(t: DataExpressionRef<'a>) -> Self {
        Self {
            queue: VecDeque::from([(t, DataPosition::empty())]),
        }
    }
}

impl<'a> Iterator for DataPositionIterator<'a> {
    type Item = (DataExpressionRef<'a>, DataPosition);

    fn next(&mut self) -> Option<Self::Item> {
        if self.queue.is_empty() {
            None
        } else {
            // Get a subterm to inspect
            let (term, pos) = self.queue.pop_front().unwrap();

            // Put subterms in the queue
            for (i, argument) in term.data_arguments().enumerate() {
                let mut new_position = pos.clone();
                new_position.push(i + 1);
                self.queue.push_back((argument, new_position));
            }

            Some((term, pos))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_data_position() {
        let t = DataExpression::from_string("f(g(a),b)").unwrap();
        let expected = DataExpression::from_string("a").unwrap();

        assert_eq!(t.get_data_position(&DataPosition::new(&[1, 1])), expected.copy());
    }

    #[test]
    fn test_data_position_iterator() {
        let t = DataExpression::from_string("f(g(a),b)").unwrap();

        for (term, pos) in DataPositionIterator::new(t.copy()) {
            assert_eq!(
                t.get_data_position(&pos),
                term,
                "The resulting (subterm, position) pair doesn't match the get_data_position implementation"
            );
        }

        assert_eq!(
            DataPositionIterator::new(t.copy()).count(),
            4,
            "The number of subterms doesn't match the expected value"
        );
    }
}
