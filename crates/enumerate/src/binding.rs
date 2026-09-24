use ahash::AHashSet;

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
use merc_data::is_data_application;
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

/// One `(variable, value)` node.
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
/// The chain is a triangular substitution: a bound variable's value may
/// mention variables bound *later* in the same chain (e.g. `v ↦ c(y1, y2)`
/// where `y1`/`y2` are fresh variables introduced to expand `v`). Use
/// [`BindingArena::resolve`] to substitute those away.
pub(crate) struct BindingArena {
    pub(crate) chain: Protected<ArenaList<Binding<'static>>>,
    pub(crate) scratch: ResolveScratch,
}

/// Buffers reused by [`BindingArena::resolve`].
#[derive(Default)]
pub(crate) struct ResolveScratch {
    /// Arguments of the application being rebuilt.
    arguments: Vec<DataExpression>,
    /// Term indices of subterms of bound values known to contain no variable.
    closed: AHashSet<usize>,
}

impl Default for BindingArena {
    fn default() -> Self {
        Self::new()
    }
}

impl BindingArena {
    pub(crate) fn new() -> Self {
        BindingArena {
            chain: Protected::new(ArenaList::default()),
            scratch: ResolveScratch::default(),
        }
    }

    /// Drops every node, keeping the backing storage's capacity. Must be
    /// called before starting a new search: every [`BindingChain`] handed out
    /// before a `clear` indexes into the discarded nodes and must not be used
    /// afterwards.
    pub(crate) fn clear(&mut self) {
        self.chain.write().clear();
        self.scratch.closed.clear();
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

    /// Applies `chain` to every variable in `vars` until no bound variable
    /// remains, rewriting each result to normal form, and stores them in `out`
    /// in the same order as `vars`. `out` is cleared first.
    ///
    /// The result is rewritten once at the end rather than per reconstructed
    /// application: substituting normal forms into a normal form can still
    /// create a redex (`@succ_nat` applied to a machine-word `Nat` digit), and
    /// rewriting at every level is quadratic in the chain depth.
    ///
    /// # Panics
    ///
    /// Panics if a variable in `vars`, or one they transitively depend on, is
    /// not bound along `chain`.
    pub(crate) fn resolve<R: RewriteEngine>(
        guard: &BindingGuard<'_>,
        rewriter: &mut R,
        scratch: &mut ResolveScratch,
        chain: BindingChain,
        vars: &[DataVariable],
        out: &mut Vec<DataExpression>,
    ) {
        out.clear();

        for v in vars {
            let value = Self::lookup_bound(guard, chain, &v.copy());
            match Self::substitute(guard, scratch, chain, &value) {
                // Values are stored in normal form, so an unchanged one is final.
                None => out.push(value.protect()),
                Some(substituted) => out.push(rewriter.rewrite(&substituted)),
            }
        }
    }

    /// Like [`Self::lookup`], but panics when `variable` is unbound.
    fn lookup_bound<'g>(
        guard: &'g BindingGuard<'_>,
        chain: BindingChain,
        variable: &DataVariableRef<'_>,
    ) -> DataExpressionRef<'g> {
        Self::lookup(guard, chain, variable).unwrap_or_else(|| panic!("{variable:?} is not bound in this BindingChain"))
    }

    /// Replaces every variable in `term` by its value along `chain`,
    /// recursively, without rewriting. Returns `None` when `term` contains no
    /// variable, which avoids a separate `is_closed` traversal and rebuilding
    /// closed subterms.
    ///
    /// Generic over [`Term`] so a recursive call can take the
    /// [`ATermRef`](merc_aterm::ATermRef) from `arguments()` without
    /// `protect`ing it first.
    fn substitute<'a, 'b, T: Term<'a, 'b>>(
        guard: &BindingGuard<'_>,
        scratch: &mut ResolveScratch,
        chain: BindingChain,
        term: &'b T,
    ) -> Option<DataExpression> {
        if is_data_variable(term) {
            let variable: DataVariableRef<'_> = term.copy().into();
            let value = Self::lookup_bound(guard, chain, &variable);
            return Some(Self::substitute(guard, scratch, chain, &value).unwrap_or_else(|| value.protect()));
        }

        // Binders never occur here: the enumerator only ever builds
        // constructor applications and plain variables.
        if !is_data_application(term) {
            return None;
        }

        if scratch.closed.contains(&term.index()) {
            return None;
        }

        // At the raw term level `arg(0)` is the head symbol and the rest are
        // the actual arguments. The unchanged arguments before the first
        // substituted one are only protected once a rebuild is certain.
        let start = scratch.arguments.len();
        let mut changed = false;
        for (index, argument) in term.arguments().skip(1).enumerate() {
            match Self::substitute(guard, scratch, chain, &argument) {
                Some(substituted) => {
                    if !changed {
                        changed = true;
                        scratch
                            .arguments
                            .extend(term.arguments().skip(1).take(index).map(|a| a.protect().into()));
                    }
                    scratch.arguments.push(substituted);
                }
                None if changed => scratch.arguments.push(argument.protect().into()),
                None => {}
            }
        }

        if !changed {
            scratch.closed.insert(term.index());
            return None;
        }

        let head: DataFunctionSymbolRef<'_> = term.arg(0).into();
        let application = DataApplication::with_args(&head, &scratch.arguments[start..]).into();
        scratch.arguments.truncate(start);
        Some(application)
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

/// The substitution `variable ↦ value`, for rewriting a body whose earlier
/// bindings were already substituted away: unlike [`ArenaSubstitution`], a
/// miss costs one comparison instead of a walk over the whole chain.
pub(crate) struct SingleSubstitution<'a> {
    variable: DataVariableRef<'a>,
    value: DataExpressionRef<'a>,
}

impl<'a> SingleSubstitution<'a> {
    pub(crate) fn new(variable: DataVariableRef<'a>, value: DataExpressionRef<'a>) -> Self {
        SingleSubstitution { variable, value }
    }
}

impl RewriteSubstitution for SingleSubstitution<'_> {
    fn get<'a>(&'a self, variable: &DataVariableRef<'_>) -> Option<DataExpressionRef<'a>> {
        (self.variable == *variable).then(|| self.value.copy())
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

    #[test]
    fn test_resolve_follows_chained_bindings() {
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

        let mut rewriter = InnermostRewriter::new(&RewriteSpecification::new(vec![]));
        let mut resolved = Vec::new();
        BindingArena::resolve(
            &guard,
            &mut rewriter,
            &mut arena.scratch,
            chain,
            std::slice::from_ref(&v),
            &mut resolved,
        );
        let expected: DataExpression = DataApplication::with_args(&c, &[a, b]).into();
        assert_eq!(resolved, vec![expected]);
    }

    #[test]
    fn test_substitute_is_linear_in_a_shared_closed_value() {
        // A one-point value is an arbitrary subterm of the body, so it can be
        // maximally shared: e_{k+1} = g(e_k, e_k) has k + 1 distinct subterms
        // but unfolds to a tree of 2^k nodes, which must not be walked.
        //
        // Calls `substitute` rather than `resolve`, since the final rewrite
        // still traverses the tree.
        let v = DataVariable::with_sort("v", nat().copy());
        let y = DataVariable::with_sort("y", nat().copy());
        let c = DataFunctionSymbol::with_sort("c", nat().copy());
        let g = DataFunctionSymbol::with_sort("g", nat().copy());
        let a: DataExpression = DataFunctionSymbol::with_sort("a", nat().copy()).into();

        let mut shared = a.clone();
        for _ in 0..64 {
            shared = DataApplication::with_args(&g, &[shared.clone(), shared]).into();
        }
        let v_value: DataExpression =
            DataApplication::with_args(&c, &[DataExpression::from(y.clone()), shared.clone()]).into();

        let mut arena = BindingArena::default();
        let mut guard = arena.chain.write();
        let chain = BindingChain::default();
        let chain = BindingArena::extend(&mut guard, chain, &v.copy(), &v_value.copy());
        let chain = BindingArena::extend(&mut guard, chain, &y.copy(), &a.copy());

        let substituted = BindingArena::substitute(&guard, &mut arena.scratch, chain, &v_value);
        let expected: DataExpression = DataApplication::with_args(&c, &[a, shared]).into();
        assert_eq!(substituted, Some(expected));
    }

    #[test]
    #[should_panic(expected = "is not bound")]
    fn test_resolve_panics_on_unbound_variable() {
        let v = DataVariable::with_sort("v", nat().copy());
        let mut resolved = Vec::new();
        let mut arena = BindingArena::default();
        let guard = arena.chain.write();
        BindingArena::resolve(
            &guard,
            &mut InnermostRewriter::new(&RewriteSpecification::new(vec![])),
            &mut arena.scratch,
            BindingChain::default(),
            &[v],
            &mut resolved,
        );
    }
}
