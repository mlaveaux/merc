use std::cell::RefCell;
use std::mem::ManuallyDrop;

use merc_aterm::ATerm;
use merc_aterm::Symb;
use merc_aterm::Symbol;
use merc_aterm::SymbolRef;
use merc_aterm::Term;
use merc_aterm::is_int_term;

use crate::BasicSort;
use crate::DataExpression;
use crate::DataFunctionSymbol;
use crate::SortExpression;

thread_local! {
    /// Thread local storage that stores various default terms representing data symbols.
    pub(crate) static DATA_SYMBOLS: RefCell<DataSymbols> = RefCell::new(DataSymbols::new());

    /// The canonical `Bool` literals `true`/`false`.
    static TRUE_LITERAL: ManuallyDrop<ATerm> = ManuallyDrop::new(bool_literal_term("true"));
    static FALSE_LITERAL: ManuallyDrop<ATerm> = ManuallyDrop::new(bool_literal_term("false"));
}

/// Builds the nullary `Bool` function symbol term named `name` (`"true"` or `"false"`).
fn bool_literal_term(name: &str) -> ATerm {
    let bool_sort = SortExpression::from(BasicSort::new("Bool"));
    let literal: DataExpression = DataFunctionSymbol::with_sort(name, bool_sort.copy()).into();
    literal.into()
}

/// Returns the canonical `Bool` literal `true`/`false`.
pub fn bool_literal(value: bool) -> DataExpression {
    if value {
        TRUE_LITERAL.with(|term| (**term).clone().into())
    } else {
        FALSE_LITERAL.with(|term| (**term).clone().into())
    }
}

/// Defines default symbols and terms for data elements.
///
/// These mirror the mCRL2 definitions since that is convenient for loading the mCRL2 binary formats.
/// The set is kept complete on purpose, so some symbols are declared here before anything reads
/// them; those carry an `allow(dead_code)`.
///
/// All `Symbol` and `ATerm` fields are wrapped in `ManuallyDrop` so that their destructors never
/// run at thread exit: a term or symbol must never be dropped after its owning thread-local term
/// pool has already been torn down, and thread-local destruction order is unspecified.
pub(crate) struct DataSymbols {
    // Sorts
    pub basic_sort_symbol: ManuallyDrop<Symbol>,
    pub function_sort_symbol: ManuallyDrop<Symbol>,
    pub container_sort_symbol: ManuallyDrop<Symbol>,
    pub structured_sort_symbol: ManuallyDrop<Symbol>,
    pub untyped_sort_symbol: ManuallyDrop<Symbol>,
    pub untyped_possible_sorts_symbol: ManuallyDrop<Symbol>,
    pub sort_alias_symbol: ManuallyDrop<Symbol>,

    // Container kinds: leaf terms in their own right, the first argument of a
    // `SortCons` rather than encoded in its symbol name (mirrors mCRL2's
    // `container_type`/`list_container`/…).
    pub list_container_symbol: ManuallyDrop<Symbol>,
    pub set_container_symbol: ManuallyDrop<Symbol>,
    pub bag_container_symbol: ManuallyDrop<Symbol>,
    pub fset_container_symbol: ManuallyDrop<Symbol>,
    pub fbag_container_symbol: ManuallyDrop<Symbol>,

    // Data expressions that are abstractions
    pub(crate) data_binder_symbol: ManuallyDrop<Symbol>,
    pub(crate) data_lambda_symbol: ManuallyDrop<Symbol>,
    pub(crate) data_exists_symbol: ManuallyDrop<Symbol>,
    pub(crate) data_forall_symbol: ManuallyDrop<Symbol>,
    pub(crate) data_set_comprehension_symbol: ManuallyDrop<Symbol>,
    pub(crate) data_bag_comprehension_symbol: ManuallyDrop<Symbol>,

    // Data expressions
    pub data_function_symbol: ManuallyDrop<Symbol>,
    pub data_function_symbol_no_index: ManuallyDrop<Symbol>,
    pub data_variable: ManuallyDrop<Symbol>,
    pub data_where_clause: ManuallyDrop<Symbol>,
    #[allow(dead_code)]
    pub data_untyped_identifier_clause: ManuallyDrop<Symbol>,

    /// The `DataEqn` symbol, for equations `condition -> lhs = rhs`. Not itself a data
    /// expression.
    pub data_equation_symbol: ManuallyDrop<Symbol>,

    /// A where-clause assignment `x := e`, used inside a `Where` term.
    pub data_whr_decl_symbol: ManuallyDrop<Symbol>,

    /// The data application symbol for a given arity.
    data_appl: Vec<Symbol>,
}

impl DataSymbols {
    fn new() -> Self {
        Self {
            basic_sort_symbol: ManuallyDrop::new(Symbol::new("SortId", 1)),
            function_sort_symbol: ManuallyDrop::new(Symbol::new("SortArrow", 2)),
            container_sort_symbol: ManuallyDrop::new(Symbol::new("SortCons", 2)),
            structured_sort_symbol: ManuallyDrop::new(Symbol::new("SortStruct", 1)),
            untyped_sort_symbol: ManuallyDrop::new(Symbol::new("UntypedSortUnknown", 0)),
            untyped_possible_sorts_symbol: ManuallyDrop::new(Symbol::new("UntypedSortsPossible", 1)),
            sort_alias_symbol: ManuallyDrop::new(Symbol::new("SortRef", 2)),

            list_container_symbol: ManuallyDrop::new(Symbol::new("SortList", 0)),
            set_container_symbol: ManuallyDrop::new(Symbol::new("SortSet", 0)),
            bag_container_symbol: ManuallyDrop::new(Symbol::new("SortBag", 0)),
            fset_container_symbol: ManuallyDrop::new(Symbol::new("SortFSet", 0)),
            fbag_container_symbol: ManuallyDrop::new(Symbol::new("SortFBag", 0)),

            data_binder_symbol: ManuallyDrop::new(Symbol::new("Binder", 3)),
            data_lambda_symbol: ManuallyDrop::new(Symbol::new("Lambda", 0)),
            data_exists_symbol: ManuallyDrop::new(Symbol::new("Exists", 0)),
            data_forall_symbol: ManuallyDrop::new(Symbol::new("Forall", 0)),
            data_set_comprehension_symbol: ManuallyDrop::new(Symbol::new("SetComp", 0)),
            data_bag_comprehension_symbol: ManuallyDrop::new(Symbol::new("BagComp", 0)),

            data_function_symbol: ManuallyDrop::new(Symbol::new("OpId", 2)),
            data_function_symbol_no_index: ManuallyDrop::new(Symbol::new("OpIdNoIndex", 2)),
            data_variable: ManuallyDrop::new(Symbol::new("DataVarId", 2)),
            data_where_clause: ManuallyDrop::new(Symbol::new("Whr", 2)),
            data_untyped_identifier_clause: ManuallyDrop::new(Symbol::new("UntypedIdentifier", 1)),
            data_equation_symbol: ManuallyDrop::new(Symbol::new("DataEqn", 4)),
            data_whr_decl_symbol: ManuallyDrop::new(Symbol::new("DataVarIdInit", 2)),

            data_appl: Vec::new(),
        }
    }

    /// Returns true iff the given term is any of the possible data expressions. This check
    /// is relatively expensive.
    pub(crate) fn is_data_expression<'a, 'b, T: Term<'a, 'b>>(&mut self, term: &'b T) -> bool {
        self.is_data_variable(term)
            || self.is_data_function_symbol(term)
            || self.is_data_machine_number(term)
            || self.is_data_binder(term)
            || self.is_data_where_clause(term)
            || self.is_data_application(term)
    }

    /// Returns true iff the given term is a data variable.
    pub(crate) fn is_data_variable<'a, 'b, T: Term<'a, 'b>>(&self, term: &'b T) -> bool {
        term.get_head_symbol() == self.data_variable.copy()
    }

    /// Returns true iff the given term is a data function symbol.
    pub(crate) fn is_data_function_symbol<'a, 'b, T: Term<'a, 'b>>(&self, term: &'b T) -> bool {
        term.get_head_symbol() == self.data_function_symbol.copy()
            || term.get_head_symbol() == self.data_function_symbol_no_index.copy()
    }

    /// Returns true iff the given term is a data machine number.
    pub(crate) fn is_data_machine_number<'a, 'b, T: Term<'a, 'b>>(&self, term: &'b T) -> bool {
        is_int_term(term)
    }

    /// Returns true iff the given term is a data where clause.
    pub(crate) fn is_data_where_clause<'a, 'b, T: Term<'a, 'b>>(&self, term: &'b T) -> bool {
        term.get_head_symbol() == self.data_where_clause.copy()
    }

    /// Returns true iff the given term is a data abstraction (binder).
    pub(crate) fn is_data_binder<'a, 'b, T: Term<'a, 'b>>(&self, term: &'b T) -> bool {
        term.get_head_symbol() == self.data_binder_symbol.copy()
    }

    /// Returns true iff the given term is a data application.
    pub(crate) fn is_data_application<'a, 'b, T: Term<'a, 'b>>(&mut self, term: &'b T) -> bool {
        let arity = term.get_head_symbol().arity();
        term.get_head_symbol() == *self.get_data_application_symbol(arity)
    }

    /// Returns the data application symbol for the given arity, creating it if necessary.
    pub(crate) fn get_data_application_symbol(&mut self, arity: usize) -> &SymbolRef<'_> {
        // It can be that data_applications are created without create_data_application in the mcrl2 ffi.
        if self.data_appl.len() <= arity {
            self.data_appl.reserve(arity + 1 - self.data_appl.len());
            while self.data_appl.len() <= arity {
                let symbol = Symbol::new("DataAppl", self.data_appl.len());
                self.data_appl.push(symbol);
            }
        }

        self.data_appl[arity].get()
    }

    /// Returns true iff the given term is any sort expression (basic, arrow, container, structured,
    /// or untyped).
    pub(crate) fn is_sort_expression<'a, 'b, T: Term<'a, 'b>>(&self, term: &'b T) -> bool {
        let sym = term.get_head_symbol();
        sym == self.basic_sort_symbol.copy()
            || sym == self.function_sort_symbol.copy()
            || sym == self.container_sort_symbol.copy()
            || sym == self.structured_sort_symbol.copy()
            || sym == self.untyped_sort_symbol.copy()
            || sym == self.untyped_possible_sorts_symbol.copy()
    }

    /// Returns true iff the given term is a basic sort.
    pub(crate) fn is_basic_sort<'a, 'b, T: Term<'a, 'b>>(&self, term: &'b T) -> bool {
        term.get_head_symbol() == self.basic_sort_symbol.copy()
    }

    /// Returns true iff the given term is a function (`SortArrow`) sort.
    pub(crate) fn is_function_sort<'a, 'b, T: Term<'a, 'b>>(&self, term: &'b T) -> bool {
        term.get_head_symbol() == self.function_sort_symbol.copy()
    }

    /// Returns true iff the given term is a container (`SortCons`) sort.
    pub(crate) fn is_container_sort<'a, 'b, T: Term<'a, 'b>>(&self, term: &'b T) -> bool {
        term.get_head_symbol() == self.container_sort_symbol.copy()
    }

    /// Returns true iff the given term is a sort alias (`SortRef`).
    pub(crate) fn is_sort_alias<'a, 'b, T: Term<'a, 'b>>(&self, term: &'b T) -> bool {
        term.get_head_symbol() == self.sort_alias_symbol.copy()
    }

    /// Returns true iff the given term is a data equation (`DataEqn`).
    pub(crate) fn is_data_equation<'a, 'b, T: Term<'a, 'b>>(&self, term: &'b T) -> bool {
        term.get_head_symbol() == self.data_equation_symbol.copy()
    }

    /// Returns true iff the given term is a where-clause assignment (`DataVarIdInit`).
    pub(crate) fn is_data_whr_decl<'a, 'b, T: Term<'a, 'b>>(&self, term: &'b T) -> bool {
        term.get_head_symbol() == self.data_whr_decl_symbol.copy()
    }
}

// Helper functions to access the DATA_SYMBOLS thread local storage.

/// See [DataSymbols::is_sort_expression].
pub(crate) fn is_sort_expression<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_sort_expression(term))
}

/// See [DataSymbols::is_basic_sort].
pub(crate) fn is_basic_sort<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_basic_sort(term))
}

/// See [DataSymbols::is_function_sort].
pub fn is_function_sort<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_function_sort(term))
}

/// See [DataSymbols::is_container_sort].
pub fn is_container_sort<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_container_sort(term))
}

/// See [DataSymbols::is_data_variable].
pub fn is_data_variable<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_variable(term))
}

/// See [DataSymbols::is_data_expression].
pub(crate) fn is_data_expression<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow_mut(|ds| ds.is_data_expression(term))
}

/// See [DataSymbols::is_data_function_symbol].
pub fn is_data_function_symbol<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_function_symbol(term))
}

/// See [DataSymbols::is_data_machine_number].
pub fn is_data_machine_number<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_machine_number(term))
}

/// See [DataSymbols::is_data_where_clause].
pub fn is_data_where_clause<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_where_clause(term))
}

/// See [DataSymbols::is_data_binder].
pub fn is_data_binder<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_binder(term))
}

/// See [DataSymbols::is_data_application].
pub fn is_data_application<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow_mut(|ds| ds.is_data_application(term))
}

/// See [DataSymbols::is_sort_alias].
pub(crate) fn is_sort_alias<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_sort_alias(term))
}

/// See [DataSymbols::is_data_equation].
pub(crate) fn is_data_equation<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_equation(term))
}

/// See [DataSymbols::is_data_whr_decl].
pub(crate) fn is_data_whr_decl<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_whr_decl(term))
}
