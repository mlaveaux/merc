
use crate::DataAbstraction;
use crate::DataApplication;
use crate::DataExpression;
use crate::DataFunctionSymbol;
use crate::DataVariable;
use crate::is_abstraction;
use crate::is_application;
use crate::is_function_symbol;
use crate::is_machine_number;
use crate::is_untyped_identifier;
use crate::is_variable;
use crate::is_where_clause;

pub trait DataExpressionVisitor {
    fn visit_variable(&mut self, var: &DataVariable) -> DataExpression {
        DataExpression::from(var.clone())
    }

    fn visit_application(&mut self, _app: &DataApplication) -> DataExpression {
        unimplemented!()
    }

    fn visit_abstraction(&mut self, _abs: &DataAbstraction) -> DataExpression {
        unimplemented!()
    }

    fn visit_function_symbol(&mut self, _fs: &DataFunctionSymbol) -> DataExpression {
        unimplemented!()
    }

    fn visit(&mut self, expr: &DataExpression) -> DataExpression {
        if is_variable(expr.get()) {
            self.visit_variable(&DataVariable::new(expr.get().clone()))
        } else if is_application(expr.get()) {
            self.visit_application(&DataApplication::new(expr.get().clone()))
        } else if is_abstraction(expr.get()) {
            self.visit_abstraction(&DataAbstraction::new(expr.get().clone()))
        } else if is_function_symbol(expr.get()) {
            self.visit_function_symbol(&DataFunctionSymbol::new(expr.get().clone()))
        } else if is_where_clause(expr.get()) {
            unimplemented!();
        } else if is_machine_number(expr.get()) {
            unimplemented!();
        } else if is_untyped_identifier(expr.get()) {
            unimplemented!();
        } else {
            unimplemented!();
        }
    }
}

/// Replaces data variables in the given data expression according to the
/// provided substitution function.
pub fn data_expression_replace_variables(
    expr: &DataExpression,
    f: &impl Fn(&DataVariable) -> DataExpression,
) -> DataExpression {
    struct ReplaceVariableBuilder<'a> {
        apply: &'a dyn Fn(&DataVariable) -> DataExpression,
    }

    impl<'a> DataExpressionVisitor for ReplaceVariableBuilder<'a> {
        fn visit_variable(&mut self, var: &DataVariable) -> DataExpression {
            (self.apply)(var)
        }
    }

    let mut builder = ReplaceVariableBuilder { apply: f };
    builder.visit(expr)
}
