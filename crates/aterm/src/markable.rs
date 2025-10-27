#![forbid(unsafe_code)]

use std::collections::VecDeque;

use crate::Marker;
use crate::aterm::ATermRef;
use crate::gc_mutex::GcMutex;

/// This trait should be used on all objects and containers related to storing unprotected terms.
pub trait Markable {
    /// Marks all the ATermRefs to prevent them from being garbage collected.
    fn mark(&self, marker: &mut Marker);

    /// Should return true iff the given term is contained in the object. Used for runtime checks.
    fn contains_term(&self, term: &ATermRef<'_>) -> bool;

    /// Returns the number of terms in the instance, used to delay garbage collection.
    fn len(&self) -> usize;

    /// Returns true iff the container is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Markable> Markable for Vec<T> {
    fn mark(&self, marker: &mut Marker) {
        for value in self {
            value.mark(marker);
        }
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        self.iter().any(|v| v.contains_term(term))
    }

    fn len(&self) -> usize {
        self.len()
    }
}

impl<T: Markable> Markable for VecDeque<T> {
    fn mark(&self, marker: &mut Marker) {
        for value in self {
            value.mark(marker);
        }
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        self.iter().any(|v| v.contains_term(term))
    }

    fn len(&self) -> usize {
        self.len()
    }
}

impl<T: Markable> Markable for GcMutex<T> {
    fn mark(&self, marker: &mut Marker) {
        self.write().mark(marker);
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        self.read().contains_term(term)
    }

    fn len(&self) -> usize {
        self.read().len()
    }
}

impl<T: Markable> Markable for Option<T> {
    fn mark(&self, marker: &mut Marker) {
        if let Some(value) = self {
            value.mark(marker);
        }
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        if let Some(value) = self {
            value.contains_term(term)
        } else {
            false
        }
    }

    fn len(&self) -> usize {
        if let Some(value) = self { value.len() } else { 0 }
    }
}
