use std::pin::Pin;

use mcrl2_sys::atermpp::ffi;

use crate::BfTermPool;
use crate::atermpp::aterm::ATermRef;

/// A type for the todo queue.
pub type Todo<'a> = Pin<&'a mut ffi::term_mark_stack>;

/// This trait should be used on all objects and containers related to storing unprotected terms.
pub trait Markable {
    /// Marks all the ATermRefs to prevent them from being garbage collected.
    fn mark(&self, todo: Todo);

    /// Should return true iff the given term is contained in the object. Used for runtime checks.
    fn contains_term(&self, term: &ATermRef<'_>) -> bool;

    /// Returns the number of terms in the instance, used to delay garbage collection.
    fn len(&self) -> usize;

    /// Returns true iff the container is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Markable for ATermRef<'_> {
    fn mark(&self, todo: Todo) {
        if !self.is_default() {
            ffi::mcrl2_aterm_mark_address(self.get(), todo);
        }
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        term == self
    }

    fn len(&self) -> usize {
        1
    }
}

impl<T: Markable> Markable for Vec<T> {
    fn mark(&self, mut todo: Todo) {
        for value in self {
            value.mark(todo.as_mut());
        }
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        self.iter().any(|v| v.contains_term(term))
    }

    fn len(&self) -> usize {
        self.len()
    }
}

impl<T: Markable + ?Sized> Markable for BfTermPool<T> {
    fn mark(&self, mut todo: Todo) {
        unsafe {
            self.get().mark(todo.as_mut());
        }
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        self.read().contains_term(term)
    }

    fn len(&self) -> usize {
        self.read().len()
    }
}
