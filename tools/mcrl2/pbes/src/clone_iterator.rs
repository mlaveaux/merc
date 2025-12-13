// This is a helper trait that is object safe and allows a type-erased iterator
// to be cloned.
pub trait CloneIterator: Iterator {
    /// Clone the iterator into a boxed trait object.
    fn clone_boxed<'a>(&self) -> Box<dyn CloneIterator<Item = Self::Item> + 'a>
    where
        Self: 'a;
}

impl<T, I> CloneIterator for I
where
    I: Iterator<Item = T> + Clone,
{
    fn clone_boxed<'a>(&self) -> Box<dyn CloneIterator<Item = Self::Item> + 'a>
    where
        Self: 'a,
    {
        Box::new(self.clone())
    }
}

impl<T: Clone + 'static> Clone for Box<dyn CloneIterator<Item = T> + '_> {
    fn clone(&self) -> Self {
        // Important! "recursive trait implementation" style
        // TODO: This I don't understand fully, but it works.
        (**self).clone_boxed()
    }
}
