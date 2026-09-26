//! Kani proofs for the soundness of the `unsafe impl Transmutable` block that
//! `#[merc_derive_terms]` generates for every `#[merc_term]` struct (see the
//! `generated: TokenStream` block in `crates/macros/src/merc_derive_terms.rs`
//! that ends in `unsafe impl Transmutable for #name_ref #generics_static`).
//!
//! # Why this lives in an integration test, not inside `merc_macros` itself
//!
//! `merc_macros` is a `proc-macro = true` crate. `cargo kani` refuses to run
//! on it at all (confirmed: `cargo kani` from `crates/macros` reports "No
//! supported targets were found" for this crate), and in any case the unsafe
//! code under review is not present in `merc_macros`'s own compiled output --
//! it exists only in the *tokens* the macro emits, which are compiled as part
//! of whichever downstream crate applies `#[merc_derive_terms]`
//! (`crates/data`, `crates/aterm`, `crates/lts`, ...).
//!
//! # Why this uses a mock `ATerm`/`ATermRef`, not a real derived term type
//!
//! Proving this against a *real* derived type (e.g. `DataExpressionRef` in
//! `crates/data`) is impractical: every real `ATermRef` is only constructible
//! through `THREAD_TERM_POOL`, a global thread-local `GcMutex`-protected term
//! pool backed by a `DashMap` and a custom freelist allocator -- far too much
//! global, allocation-heavy state for bounded model checking to explore.
//!
//! This integration test instead runs the *real* `#[merc_derive_terms]` /
//! `#[merc_term]` macros (not a hand-copied reproduction of their output)
//! against a small mock module (`support`) that supplies minimal stand-ins
//! for `ATerm`, `ATermRef`, `Term`, `Markable`, `Marker`, `SymbolRef`,
//! `ATermArgs`, `TermIterator`, `ATermIndex` and `Transmutable` -- the exact
//! set of names the generated code references. This is a genuine instantiation
//! of the macro (an integration test is an ordinary external consumer of a
//! proc-macro crate, exactly like `crates/data` is), so it exercises the
//! literal generated `unsafe impl Transmutable` block, not a paraphrase of it.
//!
//! # What the proofs check
//!
//! `std::mem::transmute::<&Self, &'a Self::Target<'a>>` only requires the
//! *reference* types to match in size, which holds for any two references to
//! `Sized` types regardless of their pointee's layout -- the built-in
//! `transmute` size check does not itself verify that `Self` and `Target<'a>`
//! agree on layout. That agreement instead relies on the Rust guarantee that
//! lifetimes are erased before layout is computed, so `TestRef<'static>` and
//! `TestRef<'a>` are literally the same compiled type. These proofs check the
//! two consequences every real caller in this codebase depends on
//! (`Return::inner`, `ProtectedReadGuard`/`ProtectedWriteGuard`'s
//! `Deref`/`DerefMut`, `SharedTerm::arguments`): that
//! `transmute_lifetime`/`transmute_lifetime_mut` read back the exact bytes
//! that were there, and that the `_mut` variant aliases the original storage
//! rather than silently operating on a copy.

#![cfg(kani)]

use std::marker::PhantomData;

use merc_macros::merc_derive_terms;
use merc_macros::merc_term;

/// Minimal stand-ins for the `merc_aterm` names `#[merc_derive_terms]`'s
/// generated code references by bare identifier. Field/method bodies that
/// the proofs below never exercise (`arg`, `arguments`, `iter`, `protect`)
/// are `unimplemented!()`: they only need to type-check, matching the real
/// trait shape in `crates/aterm/src/aterm.rs` and `crates/aterm/src/markable.rs`.
mod support {
    use std::marker::PhantomData;

    #[derive(Default)]
    pub struct Marker;

    pub trait Markable {
        fn mark(&self, marker: &mut Marker);
        fn contains_term(&self, term: &ATermRef<'_>) -> bool;
        fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool;
        fn len(&self) -> usize;
    }

    /// Local copy of `merc_aterm::Transmutable` (`crates/aterm/src/transmutable.rs`).
    /// `merc_macros`'s generated code refers to this trait by bare name; a real
    /// consumer imports the real one, this mock consumer imports this one.
    ///
    /// # Safety
    ///
    /// The implementation of this trait must ensure that the transmuted lifetime is
    /// always shorter than the original lifetime.
    pub unsafe trait Transmutable {
        type Target<'a>: ?Sized
        where
            Self: 'a;

        /// # Safety
        ///
        /// The caller must ensure that 'a does not outlive the borrow of `self`.
        unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a>;

        /// # Safety
        ///
        /// The caller must ensure that 'a does not outlive the borrow of `self`.
        unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a>;
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
    pub struct SymbolRef<'a> {
        pub id: u64,
        pub marker: PhantomData<&'a ()>,
    }

    /// Stand-in for `ATermIndex` (`StablePointer<SharedTerm>`): just a
    /// pointer-sized handle, matching what `shared()` returns a reference to.
    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
    pub struct ATermIndex(pub u64);

    /// Stand-in for the owned `ATerm`.
    #[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
    pub struct ATerm {
        pub payload: ATermIndex,
    }

    /// Stand-in for `ATermRef<'a>`: a lifetime-bearing handle whose payload
    /// does not itself depend on `'a`, matching
    /// `ATermRef<'a> { shared: ATermIndex, marker: PhantomData<&'a ()> }`.
    #[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
    pub struct ATermRef<'a> {
        pub payload: ATermIndex,
        pub marker: PhantomData<&'a ()>,
    }

    pub struct ATermArgs<'a>(PhantomData<&'a ()>);
    pub struct TermIterator<'a>(PhantomData<&'a ()>);

    pub trait Term<'a, 'b> {
        fn protect(&self) -> ATerm;
        fn arg(&'b self, index: usize) -> ATermRef<'a>;
        fn arguments(&'b self) -> ATermArgs<'a>;
        fn copy(&'b self) -> ATermRef<'a>;
        fn get_head_symbol(&'b self) -> SymbolRef<'a>;
        fn iter(&'b self) -> TermIterator<'a>;
        fn index(&self) -> usize;
        fn shared(&self) -> &ATermIndex;
    }

    impl Markable for ATerm {
        fn mark(&self, _marker: &mut Marker) {}
        fn contains_term(&self, _term: &ATermRef<'_>) -> bool {
            unimplemented!()
        }
        fn contains_symbol(&self, _symbol: &SymbolRef<'_>) -> bool {
            unimplemented!()
        }
        fn len(&self) -> usize {
            1
        }
    }

    impl<'a> Markable for ATermRef<'a> {
        fn mark(&self, _marker: &mut Marker) {}
        fn contains_term(&self, term: &ATermRef<'_>) -> bool {
            self.payload == term.payload
        }
        fn contains_symbol(&self, _symbol: &SymbolRef<'_>) -> bool {
            unimplemented!()
        }
        fn len(&self) -> usize {
            1
        }
    }

    impl<'a, 'b> Term<'a, 'b> for ATerm {
        fn protect(&self) -> ATerm {
            self.clone()
        }
        fn arg(&'b self, _index: usize) -> ATermRef<'a> {
            unimplemented!()
        }
        fn arguments(&'b self) -> ATermArgs<'a> {
            unimplemented!()
        }
        fn copy(&'b self) -> ATermRef<'a> {
            ATermRef {
                payload: self.payload,
                marker: PhantomData,
            }
        }
        fn get_head_symbol(&'b self) -> SymbolRef<'a> {
            unimplemented!()
        }
        fn iter(&'b self) -> TermIterator<'a> {
            unimplemented!()
        }
        fn index(&self) -> usize {
            self.payload.0 as usize
        }
        fn shared(&self) -> &ATermIndex {
            &self.payload
        }
    }

    impl<'a, 'b> Term<'a, 'b> for ATermRef<'a> {
        fn protect(&self) -> ATerm {
            ATerm { payload: self.payload }
        }
        fn arg(&self, _index: usize) -> ATermRef<'a> {
            unimplemented!()
        }
        fn arguments(&self) -> ATermArgs<'a> {
            unimplemented!()
        }
        fn copy(&self) -> ATermRef<'a> {
            ATermRef {
                payload: self.payload,
                marker: PhantomData,
            }
        }
        fn get_head_symbol(&self) -> SymbolRef<'a> {
            unimplemented!()
        }
        fn iter(&self) -> TermIterator<'a> {
            unimplemented!()
        }
        fn index(&self) -> usize {
            self.payload.0 as usize
        }
        fn shared(&self) -> &ATermIndex {
            &self.payload
        }
    }
}

// Runs the real `#[merc_derive_terms]`/`#[merc_term]` macros, exactly as
// `crates/data/src/data_expression.rs` does, but against the mocks above
// instead of the real `merc_aterm` types.
#[merc_derive_terms]
mod generated {
    use delegate::delegate;

    use super::merc_term;
    use super::support::*;

    /// Mirrors every real `#[merc_term]` struct in the codebase today: no
    /// generics of its own, a single `term: ATerm` field, no assertion.
    #[merc_term]
    pub struct Test {
        term: ATerm,
    }
}

use generated::Test;
use generated::TestRef;
use support::ATermIndex;
use support::ATermRef;
use support::Transmutable;

/// `transmute_lifetime`, used the way every real caller uses it (`'a` tied
/// to the borrow of `self`, e.g. `Return::inner`'s `&self -> &T::Target<'_>`),
/// must read back exactly the value that was there: the lifetime
/// reinterpretation must not corrupt or misalign the data.
#[kani::proof]
fn transmute_lifetime_preserves_value() {
    let payload: u64 = kani::any();
    let term_ref = ATermRef {
        payload: ATermIndex(payload),
        marker: PhantomData,
    };
    let value: TestRef<'static> = term_ref.into();

    // SAFETY: `'a` is inferred as the lifetime of this local borrow of
    // `value`, which outlives the reference -- the same pattern
    // `Return::inner`/`ProtectedReadGuard::deref` rely on.
    let transmuted: &TestRef<'_> = unsafe { value.transmute_lifetime() };
    assert_eq!(transmuted.term.payload.0, payload);

    // The transmute must reinterpret the existing reference, not produce one
    // to a distinct copy.
    assert_eq!(
        transmuted as *const TestRef<'_> as usize,
        &value as *const TestRef<'static> as usize
    );
}

/// `transmute_lifetime_mut` must alias the original storage: a write through
/// the transmuted `&mut` must be visible through the original binding
/// afterwards. If the macro ever transmuted *by value* instead of by
/// reference, this would instead observe two independent copies.
#[kani::proof]
fn transmute_lifetime_mut_aliases_original_storage() {
    let payload: u64 = kani::any();
    let new_payload: u64 = kani::any();
    let term_ref = ATermRef {
        payload: ATermIndex(payload),
        marker: PhantomData,
    };
    let mut value: TestRef<'static> = term_ref.into();

    {
        // SAFETY: `'a` is inferred as the lifetime of this local `&mut`
        // borrow of `value`, which does not escape this block.
        let transmuted: &mut TestRef<'_> = unsafe { value.transmute_lifetime_mut() };
        assert_eq!(transmuted.term.payload.0, payload);
        transmuted.term.payload = ATermIndex(new_payload);
    }

    assert_eq!(value.term.payload.0, new_payload);
}

/// Sanity check that the macro really did generate `Test`/`TestRef` (i.e.
/// this file exercises the real generated code, not a typo'd no-op module).
#[kani::proof]
fn generated_types_round_trip_through_conversions() {
    let payload: u64 = kani::any();
    let term = support::ATerm {
        payload: ATermIndex(payload),
    };
    let value: Test = term.into();
    let back: support::ATerm = value.into();
    assert_eq!(back.payload.0, payload);
}
