use delegate::delegate;
use merc_aterm::ATerm;
use merc_aterm::ATermArgs;
use merc_aterm::ATermIndex;
use merc_aterm::ATermList;
use merc_aterm::ATermRef;
use merc_aterm::Markable;
use merc_aterm::Symb;
use merc_aterm::Symbol;
use merc_aterm::SymbolRef;
use merc_aterm::Term;
use merc_aterm::TermIterator;
use merc_aterm::Transmutable;
use merc_aterm::storage::Marker;
use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_data::SortExpression;
use merc_macros::merc_derive_terms;
use merc_macros::merc_ignore;
use merc_macros::merc_term;

/// Checks if the given symbol represents an mCRL2 action label declaration (`ActId`).
fn is_action_label_symbol(symbol: &SymbolRef<'_>) -> bool {
    symbol.name() == "ActId" && symbol.arity() == 2
}

/// Checks if the given symbol represents an mCRL2 action instance (`Action`).
fn is_action_symbol(symbol: &SymbolRef<'_>) -> bool {
    symbol.name() == "Action" && symbol.arity() == 2
}

/// Checks if the given symbol represents an mCRL2 stochastic distribution (`Distribution`).
fn is_distribution_symbol(symbol: &SymbolRef<'_>) -> bool {
    symbol.name() == "Distribution" && symbol.arity() == 2
}

/// Checks if the given symbol represents an mCRL2 process initializer (`LinearProcessInit`).
fn is_linear_process_init_symbol(symbol: &SymbolRef<'_>) -> bool {
    symbol.name() == "LinearProcessInit" && symbol.arity() == 2
}

fn is_action_label<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_action_label_symbol(&term.get_head_symbol())
}

fn is_action<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_action_symbol(&term.get_head_symbol())
}

fn is_distribution<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_distribution_symbol(&term.get_head_symbol())
}

fn is_linear_process_init<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_linear_process_init_symbol(&term.get_head_symbol())
}

// This module is only used internally to run the proc macro.
#[merc_derive_terms]
mod inner {
    use merc_aterm::ATermStringRef;

    use super::*;

    /// The declaration of an action label: a name and the sorts of its
    /// parameters. Wire format: `ActId(name, sorts)` — arity 2.
    #[merc_term(is_action_label)]
    pub struct ActionLabel {
        term: ATerm,
    }

    impl ActionLabel {
        /// Returns the name of the action label.
        pub fn name(&self) -> ATermStringRef<'_> {
            ATermStringRef::from(self.term.arg(0))
        }

        /// Returns the sorts of the action label's parameters.
        pub fn sorts(&self) -> ATermList<SortExpression> {
            self.term.arg(1).into()
        }
    }

    /// An instance of an action: a label together with concrete argument
    /// values. Wire format: `Action(ActId, arguments)` — arity 2.
    #[merc_term(is_action)]
    pub struct Action {
        term: ATerm,
    }

    impl Action {
        /// Constructs a new action from a label and argument values, e.g. to
        /// rebuild an action after rewriting its arguments (see
        /// `LinearProcessSpecification::simplify_one_point_lps`).
        #[merc_ignore]
        pub fn new(label: ActionLabelRef<'_>, arguments: ATermList<DataExpression>) -> Self {
            let args: &[ATermRef<'_>] = &[label.into(), arguments.copy()];
            let term = ATerm::with_args(&Symbol::new("Action", 2), args);
            Action { term: term.protect() }
        }

        /// Returns the label of the action.
        pub fn label(&self) -> ActionLabelRef<'_> {
            self.term.arg(0).into()
        }

        /// Returns the data arguments of the action.
        pub fn arguments(&self) -> ATermList<DataExpression> {
            self.term.arg(1).into()
        }
    }

    /// A stochastic distribution over a set of variables. Wire format:
    /// `Distribution(variables, expr)` — arity 2.
    ///
    /// Non-stochastic specifications (the only kind this crate explores) carry
    /// an empty variable list here; the distribution expression itself is
    /// unused.
    #[merc_term(is_distribution)]
    pub struct Distribution {
        term: ATerm,
    }

    impl Distribution {
        /// Returns the distribution's bound variables.
        pub fn variables(&self) -> ATermList<DataVariable> {
            self.term.arg(0).into()
        }

        /// Returns true iff this distribution binds no variables, i.e. the
        /// specification it came from is not stochastic.
        pub fn is_trivial(&self) -> bool {
            self.variables().is_empty()
        }
    }

    /// The initial process state: the initial values of the process
    /// parameters, together with the (unused, non-stochastic) initial
    /// distribution. Wire format: `LinearProcessInit(expressions,
    /// Distribution)` — arity 2.
    #[merc_term(is_linear_process_init)]
    pub struct LinearProcessInit {
        term: ATerm,
    }

    impl LinearProcessInit {
        /// Returns the initial values of the process parameters, in
        /// declaration order.
        pub fn expressions(&self) -> ATermList<DataExpression> {
            self.term.arg(0).into()
        }

        /// Returns the (expected to be trivial) initial distribution.
        pub fn distribution(&self) -> DistributionRef<'_> {
            self.term.arg(1).into()
        }
    }
}

pub use inner::*;
