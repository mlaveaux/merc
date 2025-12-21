#[cfg(debug_assertions)]
use std::cell::RefCell;
use std::fmt::Debug;
use std::hash::Hash;
use std::mem::transmute;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::Arc;

use merc_collections::ProtectionIndex;
use merc_utilities::PhantomUnsend;

use crate::Markable;
use crate::Symb;
use crate::SymbolRef;
use crate::Term;
use crate::Transmutable;
use crate::aterm::ATermRef;
use crate::storage::GcMutex;
use crate::storage::GcMutexGuard;
use crate::storage::THREAD_TERM_POOL;

/// A container of objects, typically either terms or objects containing terms,
/// that are of trait Markable. These store ATermRef<'static> that are protected
/// during garbage collection by being in the container itself.
pub struct Protected<C> {
    container: Arc<GcMutex<C>>,
    root: ProtectionIndex,

    // Protected is not Send because it uses thread-local state for its protection
    // mechanism.
    _unsend: PhantomUnsend,
}

impl<C: Markable + Send + Transmutable + 'static> Protected<C> {
    /// Creates a new Protected container from a given container.
    pub fn new(container: C) -> Protected<C> {
        let shared = Arc::new(GcMutex::new(container));

        let root = THREAD_TERM_POOL.with_borrow(|tp| tp.protect_container(shared.clone()));

        Protected {
            container: shared,
            root,
            _unsend: Default::default(),
        }
    }

    /// Provides mutable access to the underlying container.
    pub fn write(&mut self) -> ProtectedWriteGuard<'_, C> {
        // The lifetime of ATermRef can be derived from self since it is protected by self, so transmute 'static into 'a.
        ProtectedWriteGuard::new(self.container.write())
    }

    /// Provides immutable access to the underlying container.
    pub fn read(&self) -> ProtectedReadGuard<'_, C> {
        ProtectedReadGuard::new(self.container.read())
    }
}

impl<C: Default + Markable + Send + Transmutable + 'static> Default for Protected<C> {
    fn default() -> Self {
        Protected::new(Default::default())
    }
}

impl<C: Clone + Markable + Send + Transmutable + 'static> Clone for Protected<C> {
    fn clone(&self) -> Self {
        Protected::new(self.container.read().clone())
    }
}

impl<C: Hash + Markable> Hash for Protected<C> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.container.read().hash(state)
    }
}

impl<C: PartialEq + Markable> PartialEq for Protected<C> {
    fn eq(&self, other: &Self) -> bool {
        self.container.read().eq(&other.container.read())
    }
}

impl<C: PartialOrd + Markable> PartialOrd for Protected<C> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let c: &C = &other.container.read();
        self.container.read().partial_cmp(c)
    }
}

impl<C: Debug + Markable> Debug for Protected<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let c: &C = &self.container.read();
        write!(f, "{c:?}")
    }
}

impl<C: Eq + PartialEq + Markable> Eq for Protected<C> {}
impl<C: Ord + PartialEq + PartialOrd + Markable> Ord for Protected<C> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let c: &C = &other.container.read();
        self.container.read().partial_cmp(c).unwrap()
    }
}

impl<C> Drop for Protected<C> {
    fn drop(&mut self) {
        THREAD_TERM_POOL.with_borrow(|tp| {
            tp.drop_container(self.root);
        });
    }
}

pub struct ProtectedWriteGuard<'a, C: Markable> {
    reference: GcMutexGuard<'a, C>,

    /// Terms that have been protected during the lifetime of this guard.
    #[cfg(debug_assertions)]
    protected: RefCell<Vec<ATermRef<'static>>>,

    /// Symbols that have been protected during the lifetime of this guard.
    #[cfg(debug_assertions)]
    protected_symbols: RefCell<Vec<SymbolRef<'static>>>,
}

impl<'a, C: Markable> ProtectedWriteGuard<'a, C> {
    fn new(reference: GcMutexGuard<'a, C>) -> Self {
        #[cfg(debug_assertions)]
        return ProtectedWriteGuard {
            reference,
            protected: RefCell::new(vec![]),
            protected_symbols: RefCell::new(vec![]),
        };

        #[cfg(not(debug_assertions))]
        return ProtectedWriteGuard { reference };
    }

    /// Yields a term to insert into the container.
    ///
    /// # Safety
    ///
    /// The invariant to uphold is that the resulting term MUST be inserted into the container. This is checked in debug mode, but not in release mode. If this invariant is violated, undefined behaviour may occur during garbage collection.
    /// We do not mark this function unsafe since that would make its use cumbersome.
    pub fn protect<'b>(&self, term: &'b impl Term<'a, 'b>) -> ATermRef<'static> {
        unsafe {
            // Store terms that are marked as protected to check if they are
            // actually in the container when the protection is dropped.
            #[cfg(debug_assertions)]
            self.protected
                .borrow_mut()
                .push(transmute::<ATermRef<'_>, ATermRef<'static>>(term.copy()));

            transmute::<ATermRef<'_>, ATermRef<'static>>(term.copy())
        }
    }

    /// Yields a symbol to insert into the container.
    ///
    /// The invariant to uphold is that the resulting symbol MUST be inserted into the container.
    pub fn protect_symbol<'b>(&self, symbol: &'b impl Symb<'a, 'b>) -> SymbolRef<'static> {
        unsafe {
            // Store symbols that are marked as protected to check if they are
            // actually in the container when the protection is dropped.
            #[cfg(debug_assertions)]
            self.protected_symbols
                .borrow_mut()
                .push(transmute::<SymbolRef<'_>, SymbolRef<'static>>(symbol.copy()));

            transmute::<SymbolRef<'_>, SymbolRef<'static>>(symbol.copy())
        }
    }
}

#[cfg(debug_assertions)]
impl<C: Markable> Drop for ProtectedWriteGuard<'_, C> {
    fn drop(&mut self) {
        {
            for term in self.protected.borrow().iter() {
                debug_assert!(
                    self.reference.contains_term(term),
                    "Term was protected but not actually inserted"
                );
            }

            for symbol in self.protected_symbols.borrow().iter() {
                debug_assert!(
                    self.reference.contains_symbol(symbol),
                    "Symbol was protected but not actually inserted"
                );
            }
        }
    }
}

impl<'a, C: Markable + Transmutable + 'a> Deref for ProtectedWriteGuard<'a, C> {
    type Target = C::Target<'a>;

    fn deref(&self) -> &Self::Target {
        self.reference.transmute_lifetime()
    }
}

impl<C: Markable + Transmutable> DerefMut for ProtectedWriteGuard<'_, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.reference.deref_mut().transmute_lifetime_mut()
    }
}

pub struct ProtectedReadGuard<'a, C> {
    reference: GcMutexGuard<'a, C>,
}

impl<'a, C> ProtectedReadGuard<'a, C> {
    fn new(reference: GcMutexGuard<'a, C>) -> Self {
        Self { reference }
    }
}

impl<'a, C: Transmutable> Deref for ProtectedReadGuard<'a, C> {
    type Target = C::Target<'a>;

    fn deref(&self) -> &Self::Target {
        self.reference.transmute_lifetime()
    }
}

#[cfg(test)]
mod tests {
    use crate::ATerm;

    use super::*;

    #[test]
    fn test_aterm_container() {
        let _ = merc_utilities::test_logger();

        let t = ATerm::from_string("f(g(a),b)").unwrap();

        // First test the trait for a standard container.
        let mut container = Protected::<Vec<ATermRef<'static>>>::new(vec![]);

        for _ in 0..1000 {
            let mut write = container.write();
            write.push(t.get());
        }
    }
}
