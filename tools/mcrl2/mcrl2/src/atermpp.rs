
pub struct Mcrl2ATerm {
    term: UniquePtr<aterm>,
    _marker: PhantomData<T>,
}

/// Represents a list of terms from the mCRL2 toolset.
pub struct Mcrl2AtermList<T> {
    term: UniquePtr<Mcrl2ATerm>,
    _marker: PhantomData<T>,
}