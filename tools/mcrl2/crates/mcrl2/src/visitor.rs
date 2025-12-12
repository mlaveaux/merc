use mcrl2_sys::data::ffi::mcrl2_data_expression_is_abstraction;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_application;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_function_symbol;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_machine_number;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_untyped_identifier;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_variable;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_where_clause;

use crate::DataAbstraction;
use crate::DataApplication;
use crate::DataExpression;
use crate::DataFunctionSymbol;
use crate::DataVariable;

pub trait DataExpressionVisitor {
    fn visit_variable(&mut self, var: &DataVariable) -> DataExpression {
        DataExpression::from(var.clone())
    }

    fn visit_application(&mut self, app: &DataApplication) -> DataExpression {
        unimplemented!()
    }

    fn visit_abstraction(&mut self, abs: &DataAbstraction) -> DataExpression {
        unimplemented!()
    }

    fn visit_function_symbol(&mut self, _fs: &DataFunctionSymbol) -> DataExpression {
        unimplemented!()
    }

    fn visit(&mut self, expr: &DataExpression) {
        if mcrl2_data_expression_is_variable(expr.get().get()) {
            self.visit_variable(&DataVariable::new(expr.get().clone()));
        } else if mcrl2_data_expression_is_application(expr.get().get()) {
            self.visit_application(&DataApplication::new(expr.get().clone()));
        } else if mcrl2_data_expression_is_abstraction(expr.get().get()) {
            self.visit_abstraction(&DataAbstraction::new(expr.get().clone()));
        } else if mcrl2_data_expression_is_function_symbol(expr.get().get()) {
            self.visit_function_symbol(&DataFunctionSymbol::new(expr.get().clone()));
        } else if mcrl2_data_expression_is_where_clause(expr.get().get()) {
            unimplemented!();
        } else if mcrl2_data_expression_is_machine_number(expr.get().get()) {
            unimplemented!();
        } else if mcrl2_data_expression_is_untyped_identifier(expr.get().get()) {
            unimplemented!();
        }
    }
}

/// Replaces all variables in the given expression using the provided function.
pub fn data_expression_replace_variables(expr: &DataExpression, f: &impl Fn(&DataVariable) -> DataExpression) -> DataExpression {
    struct ReplaceVariableBuilder<'a> {
        apply: &'a dyn Fn(&DataVariable) -> DataExpression,
        result: Option<DataExpression>,
    }

    impl<'a> DataExpressionVisitor for ReplaceVariableBuilder<'a> {
        fn visit_variable(&mut self, var: &DataVariable) -> DataExpression {
            (self.apply)(var)
        }
    }

    let mut builder = ReplaceVariableBuilder { apply: f, result: None };
    builder.visit(expr);
    builder.result.expect("Replacement did not occur")
}
