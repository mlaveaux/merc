use ahash::HashMap;
use ahash::HashMapExt;
use merc_aterm::Term;
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

/// A cheap, `Copy` handle into a [`BindingArena`]: the variable bindings
/// chosen so far along one branch of the enumeration search, recorded as a
/// linked list of `(variable, value)` nodes stored in the arena rather than
/// individually heap-allocated. Extending a chain ([`BindingArena::extend`])
/// is an amortized `Vec::push`, not a per-node allocation — which matters
/// because `Enumerator::drive`'s breadth-first queue holds many outstanding
/// branches at once, each extending a shared prefix of bindings on every
/// step.
///
/// A bound variable's image may itself mention a variable bound *later* in
/// the same chain (e.g. `v ↦ c(y1, y2)` where `y1`/`y2` are fresh variables
/// introduced to expand `v`, and are only bound in subsequent work items) —
/// so a chain is not by itself a valid [`RewriteSubstitution`] for splicing
/// into a term that still has *other* free variables outstanding. It is only
/// used that way (via [`BindingArena::substitution`]) to normalise the
/// search's own goal body, where the enumerator controls exactly which
/// variable is being replaced at each step. Resolving a *finished* branch's
/// bindings to fully ground values goes through [`BindingArena::resolve_all`]
/// instead, which follows the chain recursively.
#[derive(Clone, Copy, Default)]
pub(crate) struct BindingChain(Option<u32>);

struct BindingNode {
    variable: DataVariable,
    value: DataExpression,
    parent: BindingChain,
}

/// Backing store for every [`BindingChain`] produced during one search.
/// Owned by [`Enumerator`](crate::Enumerator) and [`BindingArena::clear`]ed
/// at the start of each `enumerate`/`find_witness` call, so the backing
/// `Vec`'s capacity is reused across calls instead of being reallocated from
/// scratch every time — what matters for a caller (e.g. LPS `sum`-successor
/// exploration) driving many searches through one long-lived `Enumerator`.
#[derive(Default)]
pub(crate) struct BindingArena {
    nodes: Vec<BindingNode>,
}

impl BindingArena {
    /// Drops every node, keeping the backing `Vec`'s capacity. Must be called
    /// before starting a new search: every [`BindingChain`] handed out
    /// before a `clear` indexes into the discarded nodes and must not be used
    /// afterwards.
    pub(crate) fn clear(&mut self) {
        self.nodes.clear();
    }

    /// Returns a new chain with `variable ↦ value` recorded in front of
    /// `parent`. `parent` remains valid (siblings share it).
    pub(crate) fn extend(
        &mut self,
        parent: BindingChain,
        variable: DataVariable,
        value: DataExpression,
    ) -> BindingChain {
        let index = u32::try_from(self.nodes.len()).expect("more binding-chain nodes than fit in a u32");
        self.nodes.push(BindingNode {
            variable,
            value,
            parent,
        });
        BindingChain(Some(index))
    }

    /// Returns the immediate image bound to `variable` along `chain`, if any.
    /// The image may itself still mention other variables bound later in the
    /// chain; see [`BindingArena::resolve_all`] to fully resolve it.
    ///
    /// Takes a [`DataVariableRef`] rather than an owned [`DataVariable`] so a
    /// caller holding only a reference (e.g. [`RewriteSubstitution::get`],
    /// called once per free variable of every term the search normalises)
    /// need not `protect` it — comparing two term references is a pointer
    /// comparison and never touches the protection set.
    fn lookup(&self, chain: BindingChain, variable: &DataVariableRef<'_>) -> Option<&DataExpression> {
        let mut current = chain.0;
        while let Some(index) = current {
            let node = &self.nodes[index as usize];
            if node.variable.copy() == *variable {
                return Some(&node.value);
            }
            current = node.parent.0;
        }
        None
    }

    /// Resolves every variable in `vars` to its fully ground, ready-to-report
    /// value, in the same order as `vars`, appending them to `out`.
    ///
    /// `out` is cleared first, but its capacity carries over, so a caller
    /// resolving into the same buffer on every leaf (e.g. `Enumerator::drive`'s
    /// `solution_buf`) allocates once rather than on every solution.
    ///
    /// Each variable's stored image may itself mention other bound variables
    /// (see [`BindingChain`]'s doc comment); this recursively substitutes
    /// those away, memoising each variable's resolved value so a value shared
    /// by several `vars` (or reachable along several binding paths) is only
    /// resolved once.
    ///
    /// # Panics
    ///
    /// Panics if any variable in `vars` is not bound along `chain`, which
    /// would be an [`Enumerator`](crate::Enumerator) bug: every variable
    /// passed to the search — whether eliminated by the one-point rule or
    /// consumed from the work queue — is bound by the time a branch reaches a
    /// leaf.
    pub(crate) fn resolve_all<R: RewriteEngine>(
        &self,
        rewriter: &mut R,
        chain: BindingChain,
        vars: &[DataVariable],
        out: &mut Vec<DataExpression>,
    ) {
        out.clear();
        let mut memo: HashMap<usize, DataExpression> = HashMap::new();
        for v in vars {
            let resolved = self.resolve_variable(rewriter, chain, v.copy(), &mut memo);
            out.push(resolved);
        }
    }

    /// Memoised on `Term::index` (the variable's position in the global term
    /// pool) rather than a cloned [`DataVariable`]: since terms are maximally
    /// shared, the index is already a unique, `protect`-free key — structural
    /// equality implies pointer identity, so two occurrences of the same
    /// variable share one index — which matters because this is the memo's
    /// cache-hit path, taken every time a variable recurs in the goal.
    fn resolve_variable<R: RewriteEngine>(
        &self,
        rewriter: &mut R,
        chain: BindingChain,
        variable: DataVariableRef<'_>,
        memo: &mut HashMap<usize, DataExpression>,
    ) -> DataExpression {
        if let Some(resolved) = memo.get(&variable.index()) {
            return resolved.clone();
        }

        let image = self
            .lookup(chain, &variable)
            .unwrap_or_else(|| panic!("{variable:?} is not bound in this BindingChain"))
            .clone();
        let resolved = self.resolve_term(rewriter, chain, &image, memo);
        memo.insert(variable.index(), resolved.clone());
        resolved
    }

    /// Generic over [`Term`] rather than fixed to [`DataExpression`] so a
    /// recursive call on a subterm can pass the [`ATermRef`](merc_aterm::ATermRef)
    /// yielded by `arguments()` straight through, instead of `protect`ing
    /// every argument up front just to obtain an owned value to recurse on.
    ///
    /// Rewrites every application it reconstructs before returning it:
    /// substituting an already-normal argument into a constructor can still
    /// leave the composite reducible (e.g. `@succ_nat` applied to an
    /// already-normalised machine-word `Nat` digit still needs its own
    /// carry-propagating equation to fire), so normalising only the leaves
    /// bound in the arena is not enough to guarantee the value handed back to
    /// the caller — who treats it as a [`RewriteSubstitution`] image
    /// elsewhere — is itself a normal form.
    fn resolve_term<'a, 'b, T: Term<'a, 'b>, R: RewriteEngine>(
        &self,
        rewriter: &mut R,
        chain: BindingChain,
        term: &'b T,
        memo: &mut HashMap<usize, DataExpression>,
    ) -> DataExpression {
        if is_closed(term) {
            return term.protect().into();
        }
        if is_data_variable(term) {
            let variable: DataVariableRef<'_> = term.copy().into();
            return self.resolve_variable(rewriter, chain, variable, memo);
        }

        // A term that is not closed and not a bare variable must be an
        // application (binders never occur here; the enumerator only ever
        // builds constructor applications and plain variables): at the raw
        // term level `arg(0)` is the head symbol and the rest are the actual
        // arguments, so the head needs no `protect` either — it never
        // outlives this call.
        let head: DataFunctionSymbolRef<'_> = term.arg(0).into();
        let arguments: Vec<DataExpression> = term
            .arguments()
            .skip(1)
            .map(|argument| self.resolve_term(rewriter, chain, &argument, memo))
            .collect();
        let application: DataExpression = DataApplication::with_args(&head, &arguments).into();
        rewriter.rewrite(&application)
    }

    /// Returns a [`RewriteSubstitution`] view of `chain`, usable to normalise
    /// a term in one step without building a throwaway `HashMap` per
    /// substitution — `Enumerator::drive` rewrites the search's goal body
    /// this way on every work-item expansion, so avoiding an allocation
    /// there matters.
    pub(crate) fn substitution(&self, chain: BindingChain) -> ArenaSubstitution<'_> {
        ArenaSubstitution { arena: self, chain }
    }
}

/// A [`RewriteSubstitution`] backed directly by a [`BindingArena`] chain,
/// with no intermediate collection.
pub(crate) struct ArenaSubstitution<'a> {
    arena: &'a BindingArena,
    chain: BindingChain,
}

impl RewriteSubstitution for ArenaSubstitution<'_> {
    fn get<'a>(&'a self, variable: &DataVariableRef<'_>) -> Option<DataExpressionRef<'a>> {
        self.arena.lookup(self.chain, variable).map(|value| value.copy())
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
        let chain = BindingChain::default();
        let chain = arena.extend(chain, v.clone(), v_value);
        let chain = arena.extend(chain, y1, a.clone());
        let chain = arena.extend(chain, y2, b.clone());

        let mut rewriter = rewriter();
        let mut resolved = Vec::new();
        arena.resolve_all(&mut rewriter, chain, std::slice::from_ref(&v), &mut resolved);
        let expected: DataExpression = DataApplication::with_args(&c, &[a, b]).into();
        assert_eq!(resolved, vec![expected]);
    }

    #[test]
    #[should_panic(expected = "is not bound")]
    fn test_resolve_all_panics_on_unbound_variable() {
        let v = DataVariable::with_sort("v", nat().copy());
        let mut resolved = Vec::new();
        BindingArena::default().resolve_all(&mut rewriter(), BindingChain::default(), &[v], &mut resolved);
    }
}
