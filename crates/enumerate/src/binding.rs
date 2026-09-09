use ahash::HashMap;
use ahash::HashMapExt;
use bumpalo::Bump;
use merc_aterm::ATermRef;
use merc_aterm::Markable;
use merc_aterm::Protected;
use merc_aterm::ProtectedWriteGuard;
use merc_aterm::SymbolRef;
use merc_aterm::Term;
use merc_aterm::Transmutable;
use merc_aterm::storage::Marker;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_data::DataFunctionSymbolRef;
use merc_data::DataVariable;
use merc_data::DataVariableRef;
use merc_data::is_closed;
use merc_data::is_data_variable;
use merc_sabre::RewriteEngine;
use merc_sabre::utilities::RewriteSubstitution;

use crate::arena_list::ArenaList;
use crate::arena_list::ArenaListHandle;

/// The variable bindings chosen so far along one branch of the enumeration
/// search, recorded as a linked list of `(variable, value)` nodes stored in the
/// arena. Branches share a common prefix, so extending one leaves every sibling
/// valid.
#[derive(Clone, Copy, Default)]
pub(crate) struct BindingChain(ArenaListHandle);

/// One `(variable, value)` node of a [`BindingChain`].
pub(crate) struct Binding<'a> {
    variable: DataVariableRef<'a>,
    value: DataExpressionRef<'a>,
}

/// A held write guard onto a [`BindingArena`]'s chain, obtained once per
/// search.
pub(crate) type BindingGuard<'a> = ProtectedWriteGuard<'a, ArenaList<Binding<'static>>>;

/// Backing store for every [`BindingChain`] produced during one search.
///
/// # Details
///
/// A bound variable's image may itself mention a variable bound *later* in the
/// same chain (e.g. `v ↦ c(y1, y2)` where `y1`/`y2` are fresh variables
/// introduced to expand `v`. Use [`BindingArena::resolve_all`] to obtain fully
/// ground values.
pub(crate) struct BindingArena {
    pub(crate) chain: Protected<ArenaList<Binding<'static>>>,
    /// Scratch memo table for [`BindingArena::resolve_all`].
    pub(crate) memo: HashMap<usize, DataExpression>,
    /// Scratch arena for the argument slices.
    pub(crate) scratch: Bump,
}

impl Default for BindingArena {
    fn default() -> Self {
        Self::new()
    }
}

/// 
pub(crate) struct BindingContext<'a> {
    memo: &'a mut HashMap<usize, DataExpression>,
    scratch: &'a mut Bump,
}

impl BindingArena {
    pub(crate) fn new() -> Self {
        BindingArena {
            chain: Protected::new(ArenaList::default()),
            memo: HashMap::new(),
            scratch: Bump::new(),
        }
    }

    /// Drops every node, keeping the backing storage's capacity. Must be
    /// called before starting a new search: every [`BindingChain`] handed out
    /// before a `clear` indexes into the discarded nodes and must not be used
    /// afterwards.
    pub(crate) fn clear(&mut self) {
        self.chain.write().clear();
        self.memo.clear();
    }

    /// Returns a new chain with `variable ↦ value` recorded in front of
    /// `parent`. `parent` remains valid (siblings share it).
    pub(crate) fn extend(
        guard: &mut BindingGuard<'_>,
        parent: BindingChain,
        variable: &DataVariableRef<'_>,
        value: &DataExpressionRef<'_>,
    ) -> BindingChain {
        // SAFETY: both resulting refs are inserted into `guard`'s container
        // immediately below via `push`.
        let variable_ref = unsafe { guard.protect(variable) };
        let value_ref = unsafe { guard.protect(value) };
        BindingChain(guard.push(parent.0, Binding::new(variable_ref.into(), value_ref.into())))
    }

    /// Returns the immediate image bound to `variable` along `chain`, if any.
    fn lookup<'g, 'h>(
        guard: &'g BindingGuard<'h>,
        chain: BindingChain,
        variable: &DataVariableRef<'_>,
    ) -> Option<DataExpressionRef<'g>> {
        let mut current = chain.0;
        while let Some((binding, parent)) = guard.pop(current) {
            if binding.variable.copy() == *variable {
                return Some(binding.value.copy());
            }

            current = parent;
        }
        None
    }

    /// Resolves every variable in `vars` to its fully ground, ready-to-report
    /// value, in the same order as `vars`, appending them to `out`.
    ///
    /// `out` is cleared first, but its capacity carries over.
    ///
    /// Each variable's stored image may itself mention other bound variables
    /// (see [`BindingChain`]'s doc comment); this recursively substitutes
    /// those away, memoising each variable's resolved value.
    ///
    /// # Panics
    ///
    /// Panics if any variable in `vars` is not bound along `chain`.
    pub(crate) fn resolve_all_into<R: RewriteEngine>(
        guard: &BindingGuard<'_>,
        rewriter: &mut R,
        context: &mut BindingContext<'_>,
        chain: BindingChain,
        vars: &[DataVariable],
        out: &mut Vec<DataExpression>,
    ) {
        out.clear();

        for v in vars {
            let resolved = Self::resolve_variable(&mut context.memo, &context.scratch, guard, rewriter, chain, v.copy());
            out.push(resolved);
        }
    }

    /// Memoised on `Term::index`: maximal sharing makes a variable's position
    /// in the global term pool a unique, `protect`-free key.
    fn resolve_variable<R: RewriteEngine>(
        memo: &mut HashMap<usize, DataExpression>,
        scratch: &Bump,
        guard: &BindingGuard<'_>,
        rewriter: &mut R,
        chain: BindingChain,
        variable: DataVariableRef<'_>,
    ) -> DataExpression {
        if let Some(resolved) = memo.get(&variable.index()) {
            return resolved.clone();
        }

        let image = Self::lookup(guard, chain, &variable)
            .unwrap_or_else(|| panic!("{variable:?} is not bound in this BindingChain"));
        let resolved = Self::resolve_term(guard, &mut BindingContext { memo, scratch }, rewriter, chain, &image);
        memo.insert(variable.index(), resolved.clone());
        resolved
    }

    /// Returns a normal form: every reconstructed application is rewritten
    /// again, since substituting already-normal arguments into a constructor
    /// can still leave the composite reducible (`@succ_nat` applied to a
    /// normalised machine-word `Nat` digit still needs its carry-propagating
    /// equation to fire). Callers splice the result in as a
    /// [`RewriteSubstitution`] image, which requires it.
    ///
    /// Generic over [`Term`] so a recursive call can take the
    /// [`ATermRef`](merc_aterm::ATermRef) from `arguments()` without
    /// `protect`ing it first.
    fn resolve_term<'a, 'b, T: Term<'a, 'b>, R: RewriteEngine>(
        guard: &BindingGuard<'_>,
        context: &mut BindingContext<'_>,
        rewriter: &mut R,
        chain: BindingChain,
        term: &'b T,
    ) -> DataExpression {
        if is_closed(term) {
            return term.protect().into();
        }

        if is_data_variable(term) {
            let variable: DataVariableRef<'_> = term.copy().into();
            return Self::resolve_variable(&mut context.memo, &context.scratch, guard, rewriter, chain, variable);
        }

        // A term that is not closed and not a bare variable must be an
        // application (binders never occur here; the enumerator only ever
        // builds constructor applications and plain variables): at the raw
        // term level `arg(0)` is the head symbol and the rest are the actual
        // arguments, so the head needs no `protect` either — it never
        // outlives this call.
        let head: DataFunctionSymbolRef<'_> = term.arg(0).into();
        let arguments = context.scratch.alloc_slice_fill_iter(
            term.arguments()
                .skip(1)
                .map(|argument| Self::resolve_term(guard, context, rewriter, chain, &argument)),
        );
        let application: DataExpression = DataApplication::with_args(&head, arguments).into();
        rewriter.rewrite(&application)
    }

    /// Returns a [`RewriteSubstitution`] view of `chain`, with no intervening
    /// collection to build.
    pub(crate) fn substitution<'g, 'h>(guard: &'g BindingGuard<'h>, chain: BindingChain) -> ArenaSubstitution<'g, 'h> {
        ArenaSubstitution::new(guard, chain)
    }
}

impl<'a> Binding<'a> {
    fn new(variable: DataVariableRef<'a>, value: DataExpressionRef<'a>) -> Self {
        Binding { variable, value }
    }
}

impl Markable for Binding<'_> {
    fn mark(&self, marker: &mut Marker) {
        self.variable.mark(marker);
        self.value.mark(marker);
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        self.variable.contains_term(term) || self.value.contains_term(term)
    }

    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
        self.variable.contains_symbol(symbol) || self.value.contains_symbol(symbol)
    }

    fn len(&self) -> usize {
        2
    }
}

// SAFETY: `Binding<'a>` is a `#[repr(Rust)]` struct of two lifetime-erasable
// ref handles; this mirrors `merc_derive_terms`'s own
// `unsafe impl Transmutable for #name_ref<'static>` (one field), just over
// two fields instead of one.
unsafe impl Transmutable for Binding<'static> {
    type Target<'a> = Binding<'a>;

    unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        // SAFETY: see the impl comment above.
        unsafe { std::mem::transmute::<&Self, &'a Binding<'a>>(self) }
    }

    unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        // SAFETY: see the impl comment above.
        unsafe { std::mem::transmute::<&mut Self, &'a mut Binding<'a>>(self) }
    }
}

/// A [`RewriteSubstitution`] backed directly by a [`BindingArena`] chain.
pub(crate) struct ArenaSubstitution<'g, 'h> {
    guard: &'g BindingGuard<'h>,
    chain: BindingChain,
}

impl<'g, 'h> ArenaSubstitution<'g, 'h> {
    fn new(guard: &'g BindingGuard<'h>, chain: BindingChain) -> Self {
        ArenaSubstitution { guard, chain }
    }
}

impl RewriteSubstitution for ArenaSubstitution<'_, '_> {
    fn get<'a>(&'a self, variable: &DataVariableRef<'_>) -> Option<DataExpressionRef<'a>> {
        BindingArena::lookup(self.guard, self.chain, variable)
    }
}

#[cfg(test)]
mod tests {
    use merc_data::BasicSort;
    use merc_data::DataApplication;
    use merc_data::DataExpression;
    use merc_data::DataFunctionSymbol;
    use merc_data::DataVariable;
    use merc_data::SortExpression;
    use merc_sabre::InnermostRewriter;
    use merc_sabre::RewriteSpecification;

    use super::BindingArena;
    use super::BindingChain;

    fn nat() -> SortExpression {
        SortExpression::from(BasicSort::new("Nat"))
    }

    /// `c`/`a`/`b` below are uninterpreted (no rewrite rules), so an empty
    /// specification's rewriter is a no-op on them — used only so
    /// `resolve_all` has a [`merc_sabre::RewriteEngine`] to normalise the
    /// composites it reconstructs.
    fn rewriter() -> InnermostRewriter {
        InnermostRewriter::new(&RewriteSpecification::new(vec![]))
    }

    #[test]
    fn test_resolve_all_follows_chained_bindings() {
        // v -> c(y1, y2), y1 -> a, y2 -> b: resolving v must recursively
        // substitute y1 and y2, which are only bound *later* in the chain.
        let v = DataVariable::with_sort("v", nat().copy());
        let y1 = DataVariable::with_sort("y1", nat().copy());
        let y2 = DataVariable::with_sort("y2", nat().copy());
        let c = DataFunctionSymbol::with_sort("c", nat().copy());
        let a: DataExpression = DataFunctionSymbol::with_sort("a", nat().copy()).into();
        let b: DataExpression = DataFunctionSymbol::with_sort("b", nat().copy()).into();

        let v_value: DataExpression = DataApplication::with_args(
            &c,
            &[DataExpression::from(y1.clone()), DataExpression::from(y2.clone())],
        )
        .into();

        let mut arena = BindingArena::default();
        let mut guard = arena.chain.write();
        let chain = BindingChain::default();
        let chain = BindingArena::extend(&mut guard, chain, &v.copy(), &v_value.copy());
        let chain = BindingArena::extend(&mut guard, chain, &y1.copy(), &a.copy());
        let chain = BindingArena::extend(&mut guard, chain, &y2.copy(), &b.copy());

        let mut rewriter = rewriter();
        let mut resolved = Vec::new();
        BindingArena::resolve_all_into(
            &mut arena.memo,
            &mut arena.scratch,
            &guard,
            &mut rewriter,
            chain,
            std::slice::from_ref(&v),
            &mut resolved,
        );
        let expected: DataExpression = DataApplication::with_args(&c, &[a, b]).into();
        assert_eq!(resolved, vec![expected]);
    }

    #[test]
    #[should_panic(expected = "is not bound")]
    fn test_resolve_all_panics_on_unbound_variable() {
        let v = DataVariable::with_sort("v", nat().copy());
        let mut resolved = Vec::new();
        let mut arena = BindingArena::default();
        let guard = arena.chain.write();
        BindingArena::resolve_all_into(
            &mut arena.memo,
            &mut arena.scratch,
            &guard,
            &mut rewriter(),
            BindingChain::default(),
            &[v],
            &mut resolved,
        );
    }
}
