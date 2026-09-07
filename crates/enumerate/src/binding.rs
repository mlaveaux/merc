use std::rc::Rc;

use ahash::HashMap;
use ahash::HashMapExt;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_data::DataVariable;
use merc_data::DataVariableRef;
use merc_data::is_closed;
use merc_data::is_data_variable;
use merc_sabre::utilities::RewriteSubstitution;

/// An immutable, cheaply-extended record of the variable bindings chosen so
/// far along one branch of the enumeration search.
///
/// A bound variable's image may itself mention a variable bound *later* in
/// the same chain (e.g. `v ↦ c(y1, y2)` where `y1`/`y2` are fresh variables
/// introduced to expand `v`, and are only bound in subsequent work items) —
/// so a chain is not by itself a valid [`RewriteSubstitution`] for splicing
/// into a term that still has *other* free variables outstanding. It is only
/// used that way (via `RewriteSubstitution::get`) to normalise the search's
/// own goal body, where the enumerator controls exactly which variable is
/// being replaced at each step. Resolving a *finished* branch's bindings to
/// fully ground values goes through [`BindingChain::resolve_all`] instead,
/// which follows the chain recursively.
#[derive(Clone, Default)]
pub(crate) struct BindingChain(Option<Rc<BindingNode>>);

struct BindingNode {
    variable: DataVariable,
    value: DataExpression,
    parent: BindingChain,
}

impl BindingChain {
    /// Returns a new chain with `variable ↦ value` recorded in front of
    /// `self`. `self` is left unchanged (siblings share it).
    pub(crate) fn extend(&self, variable: DataVariable, value: DataExpression) -> BindingChain {
        BindingChain(Some(Rc::new(BindingNode {
            variable,
            value,
            parent: self.clone(),
        })))
    }

    /// Returns the immediate image bound to `variable`, if any. The image may
    /// itself still mention other variables bound later in the chain; see
    /// [`BindingChain::resolve_all`] to fully resolve it.
    fn lookup(&self, variable: &DataVariable) -> Option<&DataExpression> {
        let mut node = &self.0;
        while let Some(n) = node {
            if &n.variable == variable {
                return Some(&n.value);
            }
            node = &n.parent.0;
        }
        None
    }

    /// Resolves every variable in `vars` to its fully ground, ready-to-report
    /// value, in the same order as `vars`.
    ///
    /// Each variable's stored image may itself mention other bound variables
    /// (see the type-level doc comment); this recursively substitutes those
    /// away, memoising each variable's resolved value so a value shared by
    /// several `vars` (or reachable along several binding paths) is only
    /// resolved once.
    ///
    /// # Panics
    ///
    /// Panics if any variable in `vars` is not bound in this chain, which
    /// would be an [`Enumerator`](crate::Enumerator) bug: every variable
    /// passed to the search — whether eliminated by the one-point rule or
    /// consumed from the work queue — is bound by the time a branch reaches a
    /// leaf.
    pub(crate) fn resolve_all(&self, vars: &[DataVariable]) -> Vec<DataExpression> {
        let mut memo: HashMap<DataVariable, DataExpression> = HashMap::new();
        vars.iter().map(|v| self.resolve_variable(v, &mut memo)).collect()
    }

    fn resolve_variable(
        &self,
        variable: &DataVariable,
        memo: &mut HashMap<DataVariable, DataExpression>,
    ) -> DataExpression {
        if let Some(resolved) = memo.get(variable) {
            return resolved.clone();
        }

        let image = self
            .lookup(variable)
            .unwrap_or_else(|| panic!("{variable} is not bound in this BindingChain"))
            .clone();
        let resolved = self.resolve_term(&image, memo);
        memo.insert(variable.clone(), resolved.clone());
        resolved
    }

    fn resolve_term(&self, term: &DataExpression, memo: &mut HashMap<DataVariable, DataExpression>) -> DataExpression {
        if is_closed(term) {
            return term.clone();
        }
        if is_data_variable(term) {
            let variable: DataVariable = term.clone().into();
            return self.resolve_variable(&variable, memo);
        }

        // A term that is not closed and not a bare variable must be an
        // application (binders never occur here; the enumerator only ever
        // builds constructor applications and plain variables).
        let head = term.data_function_symbol().protect();
        let arguments: Vec<DataExpression> = term
            .data_arguments()
            .map(|argument| self.resolve_term(&argument.protect(), memo))
            .collect();
        DataApplication::with_args(&head, &arguments).into()
    }
}

impl RewriteSubstitution for BindingChain {
    fn get<'a>(&'a self, variable: &DataVariableRef<'_>) -> Option<DataExpressionRef<'a>> {
        self.lookup(&variable.protect()).map(|value| value.copy())
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

    use super::BindingChain;

    fn nat() -> SortExpression {
        SortExpression::from(BasicSort::new("Nat"))
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

        let chain = BindingChain::default()
            .extend(v.clone(), v_value)
            .extend(y1, a.clone())
            .extend(y2, b.clone());

        let resolved = chain.resolve_all(std::slice::from_ref(&v));
        let expected: DataExpression = DataApplication::with_args(&c, &[a, b]).into();
        assert_eq!(resolved, vec![expected]);
    }

    #[test]
    #[should_panic(expected = "is not bound")]
    fn test_resolve_all_panics_on_unbound_variable() {
        let v = DataVariable::with_sort("v", nat().copy());
        BindingChain::default().resolve_all(&[v]);
    }
}
