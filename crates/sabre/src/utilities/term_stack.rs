use std::fmt;

use ahash::HashMap;
use ahash::HashMapExt;
use merc_aterm::ATermRef;
use merc_aterm::Markable;
use merc_aterm::Protected;
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
use merc_data::is_data_machine_number;
use merc_data::is_data_variable;
use merc_utilities::debug_trace;

use crate::Rule;
use crate::utilities::InnermostStack;

use super::DataPosition;
use super::DataPositionIterator;

/// A stack used to represent a term with free variables that can be constructed
/// efficiently.
///
/// It stores as much as possible in the term pool. Due to variables it cannot
/// be fully compressed. For variables it stores the position in the lhs of a
/// rewrite rule where the concrete term can be found that will replace the
/// variable.
///
#[derive(Hash, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TermStack {
    /// The innermost rewrite stack for the right hand side and the positions that must be added to the stack.
    pub innermost_stack: Protected<Vec<Config<'static>>>,
    /// The variables of the left-hand side that must be placed at certain places in the stack.
    pub variables: Vec<(DataPosition, usize)>,
    /// The number of elements that must be reserved on the innermost stack.
    pub stack_size: usize,
}

#[derive(Hash, Eq, PartialEq, Ord, PartialOrd, Debug)]
pub enum Config<'a> {
    /// Rewrite the top of the stack and put result at the given index.
    Rewrite(usize),
    /// Constructs function symbol with given arity at the given index.
    Construct(DataFunctionSymbolRef<'a>, usize, usize),
    /// A concrete term to be placed at the current position in the stack.
    Term(DataExpressionRef<'a>, usize),
    /// Yields the given index as returned term.
    Return(),
}

impl Markable for Config<'_> {
    fn mark(&self, marker: &mut Marker<'_>) {
        if let Config::Construct(t, _, _) = self {
            t.mark(marker);
        }
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        if let Config::Construct(t, _, _) = self {
            t.contains_term(term)
        } else {
            false
        }
    }

    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
        if let Config::Construct(t, _, _) = self {
            t.contains_symbol(symbol)
        } else {
            false
        }
    }

    fn len(&self) -> usize {
        if let Config::Construct(_, _, _) = self { 1 } else { 0 }
    }
}

impl Transmutable for Config<'static> {
    type Target<'a> = Config<'a>;

    fn transmute_lifetime<'a>(&'_ self) -> &'a Self::Target<'a> {
        unsafe { std::mem::transmute::<&Self, &'a Config>(self) }
    }

    fn transmute_lifetime_mut<'a>(&'_ mut self) -> &'a mut Self::Target<'a> {
        unsafe { std::mem::transmute::<&mut Self, &'a mut Config>(self) }
    }
}

impl fmt::Display for Config<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Config::Rewrite(result) => write!(f, "Rewrite({result})"),
            Config::Construct(symbol, arity, result) => {
                write!(f, "Construct({symbol}, {arity}, {result})")
            }
            Config::Term(term, result) => {
                write!(f, "Term({term}, {result})")
            }
            Config::Return() => write!(f, "Return()"),
        }
    }
}

impl TermStack {
    /// Construct a new right-hand stack for a given equation/rewrite rule.
    pub fn new(rule: &Rule) -> TermStack {
        Self::from_term(&rule.rhs.copy(), &create_var_map(&rule.lhs))
    }

    /// Construct a term stack from a data expression where variables are taken from a specific position of the left hand side.
    pub fn from_term(term: &DataExpressionRef, var_map: &HashMap<DataVariable, DataPosition>) -> TermStack {
        // Compute the extra information for the InnermostRewriter.
        let mut innermost_stack: Protected<Vec<Config>> = Protected::new(vec![]);
        let mut variables = vec![];
        let mut stack_size = 0;

        for (term, _position) in DataPositionIterator::new(term.copy()) {
            if is_data_variable(&term) {
                let variable: DataVariableRef<'_> = term.into();
                variables.push((
                    var_map
                        .get(&variable.protect())
                        .expect("All variables in the right hand side must occur in the left hand side")
                        .clone(),
                    stack_size,
                ));
                stack_size += 1;
            } else if is_data_machine_number(&term) {
                // Skip SortId(@NoValue) and OpId
            } else {
                let arity = term.data_arguments().len();
                let mut write = innermost_stack.write();
                write.push(Config::Construct(term.data_function_symbol(), arity, stack_size));
                stack_size += 1;
            }
        }

        TermStack {
            innermost_stack,
            stack_size,
            variables,
        }
    }

    // See [evaluate_with]
    pub fn evaluate<'a, 'b>(&self, term: &'b impl Term<'a, 'b>) -> DataExpression {
        let mut builder = TermStackBuilder::new();
        self.evaluate_with(term, &mut builder)
    }

    /// Evaluate the rhs stack for the given term and returns the result.
    pub fn evaluate_with<'a, 'b>(&self, term: &'b impl Term<'a, 'b>, builder: &mut TermStackBuilder) -> DataExpression {
        let stack = &mut builder.stack;
        {
            let mut write = stack.terms.write();
            write.clear();
            write.push(None);
        }

        InnermostStack::integrate(
            &mut stack.configs.write(),
            &mut stack.terms.write(),
            self,
            &DataExpressionRef::from(term.copy()),
            0,
        );

        loop {
            debug_trace!("{}", stack);

            let mut write_configs = stack.configs.write();
            if let Some(config) = write_configs.pop() {
                match config {
                    Config::Construct(symbol, arity, index) => {
                        // Take the last arity arguments.
                        let mut write_terms = stack.terms.write();
                        let length = write_terms.len();

                        let arguments = &write_terms[length - arity..];

                        let term: DataExpression = if arguments.is_empty() {
                            symbol.protect().into()
                        } else {
                            DataApplication::with_iter(&symbol.copy(), arguments.len(), arguments.iter().flatten())
                                .into()
                        };

                        // Add the term on the stack.
                        write_terms.drain(length - arity..);
                        let t = write_terms.protect(&term);
                        write_terms[index] = Some(t.into());
                    }
                    Config::Term(term, index) => {
                        let mut write_terms = stack.terms.write();
                        let t = write_terms.protect(&term);
                        write_terms[index] = Some(t.into());
                    }
                    Config::Rewrite(_) => {
                        unreachable!("This case should not happen");
                    }
                    Config::Return() => {
                        unreachable!("This case should not happen");
                    }
                }
            } else {
                break;
            }
        }

        debug_assert!(
            stack.terms.read().len() == 1,
            "Expect exactly one term on the result stack"
        );

        let mut write_terms = stack.terms.write();

        write_terms
            .pop()
            .expect("The result should be the last element on the stack")
            .expect("The result should be Some")
            .protect()
    }

    /// Used to check if a subterm is duplicated, for example "times(s(x), y) =
    /// plus(y, times(x,y))" is duplicating.
    pub(crate) fn contains_duplicate_var_references(&self) -> bool {
        let mut variables: Vec<&DataPosition> = self.variables.iter().map(|(v, _)| v).collect();

        // Check if there are duplicates.
        variables.sort_unstable();
        let len = variables.len();
        variables.dedup();

        len != variables.len()
    }
}

impl Clone for TermStack {
    fn clone(&self) -> Self {
        // TODO: It would make sense if Protected could implement Clone.
        let mut innermost_stack: Protected<Vec<Config>> = Protected::new(vec![]);

        let read = self.innermost_stack.read();
        let mut write = innermost_stack.write();
        for t in read.iter() {
            match t {
                Config::Rewrite(x) => write.push(Config::Rewrite(*x)),
                Config::Construct(f, x, y) => {
                    write.push(Config::Construct(f.copy(), *x, *y));
                }
                Config::Term(t, y) => {
                    write.push(Config::Term(t.copy(), *y));
                }
                Config::Return() => write.push(Config::Return()),
            }
        }
        drop(write);

        Self {
            variables: self.variables.clone(),
            stack_size: self.stack_size,
            innermost_stack,
        }
    }
}

pub struct TermStackBuilder {
    stack: InnermostStack,
}

impl TermStackBuilder {
    pub fn new() -> Self {
        Self {
            stack: InnermostStack::default(),
        }
    }
}

impl Default for TermStackBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a mapping of variables to their position in the given term
pub fn create_var_map(t: &DataExpression) -> HashMap<DataVariable, DataPosition> {
    let mut result = HashMap::new();

    for (term, position) in DataPositionIterator::new(t.copy()) {
        if is_data_variable(&term) {
            result.insert(term.protect().into(), position.clone());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    use ahash::AHashSet;
    use merc_data::DataFunctionSymbol;
    use merc_utilities::test_logger;

    use crate::test_utility::create_rewrite_rule;

    use test_log::test;

    #[test]
    fn test_rhs_stack() {
        let rhs_stack = TermStack::new(&create_rewrite_rule("fact(s(N))", "times(s(N), fact(N))", &["N"]).unwrap());
        let mut expected = Protected::new(vec![]);

        let t1 = DataFunctionSymbol::new("times");
        let t2 = DataFunctionSymbol::new("s");
        let t3 = DataFunctionSymbol::new("fact");

        let mut write = expected.write();
        write.push(Config::Construct(t1.copy(), 2, 0));
        write.push(Config::Construct(t2.copy(), 1, 1));
        write.push(Config::Construct(t3.copy(), 1, 2));
        drop(write);

        // Check if the resulting construction succeeded.
        assert_eq!(
            rhs_stack.innermost_stack, expected,
            "The resulting config stack is not as expected"
        );

        assert_eq!(rhs_stack.stack_size, 5, "The stack size does not match");

        // Test the evaluation
        let lhs = DataExpression::from_string("fact(s(a))").unwrap();
        let rhs = DataExpression::from_string("times(s(a), fact(a))").unwrap();

        assert_eq!(
            rhs_stack.evaluate(&lhs),
            rhs,
            "The rhs stack does not evaluate to the expected term"
        );
    }

    #[test]
    fn test_rhs_stack_variable() {
        let rhs = TermStack::new(&create_rewrite_rule("f(x)", "x", &["x"]).unwrap());

        // Check if the resulting construction succeeded.
        assert!(
            rhs.innermost_stack.read().is_empty(),
            "The resulting config stack is not as expected"
        );

        assert_eq!(rhs.stack_size, 1, "The stack size does not match");
    }

    #[test]
    fn test_evaluation() {
        test_logger();

        let rhs = DataExpression::from_string_untyped("f(f(a,a),x)", &AHashSet::from([String::from("x")])).unwrap();
        let lhs = DataExpression::from_string("g(b)").unwrap();

        // Make a variable map with only x@1.
        let mut map = HashMap::new();
        map.insert(DataVariable::new("x"), DataPosition::new(&[1]));

        let sctt = TermStack::from_term(&rhs.copy(), &map);

        let expected = DataExpression::from_string("f(f(a,a),b)").unwrap();

        assert_eq!(sctt.evaluate(&lhs), expected);
    }

    #[test]
    fn test_create_varmap() {
        let t = DataExpression::from_string_untyped("f(x,x)", &AHashSet::from([String::from("x")])).unwrap();
        let x = DataVariable::new("x");

        let map = create_var_map(&t);
        assert!(map.contains_key(&x));
    }

    #[test]
    fn test_is_duplicating() {
        let rhs = DataExpression::from_string_untyped("f(x,x)", &AHashSet::from([String::from("x")])).unwrap();

        // Make a variable map with only x@1.
        let mut map = HashMap::new();
        map.insert(DataVariable::new("x"), DataPosition::new(&[1]));

        let sctt = TermStack::from_term(&rhs.copy(), &map);
        assert!(sctt.contains_duplicate_var_references(), "This sctt is duplicating");
    }
}
