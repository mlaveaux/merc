use std::cell::RefCell;
use std::mem::ManuallyDrop;

use merc_aterm::Symb;
use merc_aterm::Symbol;
use merc_aterm::SymbolRef;
use merc_aterm::Term;
use merc_aterm::is_int_term;

thread_local! {
    /// Thread local storage that stores various default terms representing data symbols.
    pub static DATA_SYMBOLS: RefCell<DataSymbols> = RefCell::new(DataSymbols::new());
}

/// Defines default symbols and terms for data elements.
///
/// For now these mirror the mCRL2 definitions since that is convenient.
pub struct DataSymbols {
    pub sort_id_symbol: ManuallyDrop<Symbol>,
    /// OpId(name, sort)
    pub data_function_symbol: ManuallyDrop<Symbol>,
    pub data_function_symbol_no_index: ManuallyDrop<Symbol>,
    pub data_variable: ManuallyDrop<Symbol>,
    pub data_where_clause: ManuallyDrop<Symbol>,
    pub data_abstraction: ManuallyDrop<Symbol>,

    /// The data application symbol for a given arity.
    data_appl: Vec<Symbol>,
}

impl DataSymbols {
    fn new() -> Self {
        Self {
            sort_id_symbol: ManuallyDrop::new(Symbol::new("SortId", 1)),
            data_function_symbol: ManuallyDrop::new(Symbol::new("OpId", 2)),
            data_function_symbol_no_index: ManuallyDrop::new(Symbol::new("OpIdNoIndex", 2)),
            data_variable: ManuallyDrop::new(Symbol::new("DataVarId", 2)),

            data_where_clause: ManuallyDrop::new(Symbol::new("Where", 2)),
            data_abstraction: ManuallyDrop::new(Symbol::new("Abstraction", 2)),
            data_appl: Vec::new(),
        }
    }

    pub fn is_sort_expression<'a, 'b>(&self, term: &'b impl Term<'a, 'b>) -> bool {
        term.get_head_symbol() == **self.sort_id_symbol
    }

    pub fn is_bool_sort<'a, 'b>(&self, _term: &'b impl Term<'a, 'b>) -> bool {
        true
    }

    pub fn is_data_variable<'a, 'b>(&self, term: &'b impl Term<'a, 'b>) -> bool {
        term.get_head_symbol() == **self.data_variable
    }

    pub fn is_data_expression<'a, 'b>(&mut self, term: &'b impl Term<'a, 'b>) -> bool {
        self.is_data_variable(term)
            || self.is_data_function_symbol(term)
            || self.is_data_machine_number(term)
            || self.is_data_abstraction(term)
            || self.is_data_where_clause(term)
            || self.is_data_application(term)
    }

    pub fn is_data_function_symbol<'a, 'b>(&self, term: &'b impl Term<'a, 'b>) -> bool {
        term.get_head_symbol() == **self.data_function_symbol
            || term.get_head_symbol() == **self.data_function_symbol_no_index
    }

    pub fn is_data_machine_number<'a, 'b>(&self, term: &'b impl Term<'a, 'b>) -> bool {
        is_int_term(term)
    }

    pub fn is_data_where_clause<'a, 'b>(&self, term: &'b impl Term<'a, 'b>) -> bool {
        term.get_head_symbol() == **self.data_where_clause
    }

    pub fn is_data_abstraction<'a, 'b>(&self, term: &'b impl Term<'a, 'b>) -> bool {
        term.get_head_symbol() == **self.data_abstraction
    }

    /// Returns true iff the given term is a data application.
    pub fn is_data_application<'a, 'b>(&self, term: &'b impl Term<'a, 'b>) -> bool {
        if let Some(symbol) = self.data_appl.get(term.get_head_symbol().arity()) {
            return term.get_head_symbol() == **symbol;
        }

        false
    }

    pub fn get_data_application_symbol(&mut self, arity: usize) -> &SymbolRef<'_> {
        // It can be that data_applications are created without create_data_application in the mcrl2 ffi.
        while self.data_appl.len() <= arity {
            let symbol = Symbol::new("DataAppl", self.data_appl.len());

            self.data_appl.push(symbol);
        }

        &self.data_appl[arity]
    }
}

pub fn is_sort_expression<'a, 'b>(term: &'b impl Term<'a, 'b>) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_sort_expression(term))
}

pub fn is_bool_sort<'a, 'b>(term: &'b impl Term<'a, 'b>) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_bool_sort(term))
}

pub fn is_data_variable<'a, 'b>(term: &'b impl Term<'a, 'b>) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_variable(term))
}

pub fn is_data_expression<'a, 'b>(term: &'b impl Term<'a, 'b>) -> bool {
    DATA_SYMBOLS.with_borrow_mut(|ds| ds.is_data_expression(term))
}

pub fn is_data_function_symbol<'a, 'b>(term: &'b impl Term<'a, 'b>) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_function_symbol(term))
}

pub fn is_data_machine_number<'a, 'b>(term: &'b impl Term<'a, 'b>) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_machine_number(term))
}

pub fn is_data_where_clause<'a, 'b>(term: &'b impl Term<'a, 'b>) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_where_clause(term))
}

pub fn is_data_abstraction<'a, 'b>(term: &'b impl Term<'a, 'b>) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_abstraction(term))
}

pub fn is_data_application<'a, 'b>(term: &'b impl Term<'a, 'b>) -> bool {
    DATA_SYMBOLS.with_borrow(|ds| ds.is_data_application(term))
}
