use crate::DataAbstractionRef;
use crate::DataApplicationRef;
use crate::DataExpression;
use crate::DataExpressionRef;
use crate::DataFunctionSymbolRef;
use crate::DataMachineNumberRef;
use crate::DataUntypedIdentifierRef;
use crate::DataVariableRef;
use crate::DataWhereClauseRef;
use crate::PbesAndRef;
use crate::PbesExistsRef;
use crate::PbesExpression;
use crate::PbesExpressionRef;
use crate::PbesForallRef;
use crate::PbesImpRef;
use crate::PbesNotRef;
use crate::PbesOrRef;
use crate::PbesPropositionalVariableInstantiation;
use crate::PbesPropositionalVariableInstantiationRef;
use crate::is_abstraction;
use crate::is_application;
use crate::is_function_symbol;
use crate::is_machine_number;
use crate::is_pbes_and;
use crate::is_pbes_exists;
use crate::is_pbes_forall;
use crate::is_pbes_imp;
use crate::is_pbes_not;
use crate::is_pbes_or;
use crate::is_pbes_propositional_variable_instantiation;
use crate::is_untyped_identifier;
use crate::is_variable;
use crate::is_where_clause;

pub trait DataExpressionVisitor {
    fn visit_variable(&mut self, _var: &DataVariableRef<'_>) -> Option<DataExpression> {
        None
    }

    fn visit_application(&mut self, appl: &DataApplicationRef<'_>) -> Option<DataExpression> {
        let _head = self.visit(&appl.data_function_symbol().into());

        appl.data_arguments().for_each(|arg| {
            self.visit(&arg.into());
        });

        None
    }

    fn visit_abstraction(&mut self, abstraction: &DataAbstractionRef<'_>) -> Option<DataExpression> {
        let _body = self.visit(&abstraction.body());
        None
    }

    fn visit_function_symbol(&mut self, _function_symbol: &DataFunctionSymbolRef<'_>) -> Option<DataExpression> {
        None
    }

    fn visit_where_clause(&mut self, where_: &DataWhereClauseRef<'_>) -> Option<DataExpression> {
        let _body = self.visit(&where_.body());
        None
    }

    fn visit_machine_number(&mut self, _number: &DataMachineNumberRef<'_>) -> Option<DataExpression> {
        None
    }

    fn visit_untyped_identifier(&mut self, _identifier: &DataUntypedIdentifierRef<'_>) -> Option<DataExpression> {
        None
    }

    fn visit(&mut self, expr: &DataExpressionRef<'_>) -> Option<DataExpression> {
        if is_variable(&expr.copy()) {
            self.visit_variable(&DataVariableRef::from(expr.copy()))
        } else if is_application(&expr.copy()) {
            self.visit_application(&DataApplicationRef::from(expr.copy()))
        } else if is_abstraction(&expr.copy()) {
            self.visit_abstraction(&DataAbstractionRef::from(expr.copy()))
        } else if is_function_symbol(&expr.copy()) {
            self.visit_function_symbol(&DataFunctionSymbolRef::from(expr.copy()))
        } else if is_where_clause(&expr.copy()) {
            self.visit_where_clause(&DataWhereClauseRef::from(expr.copy()))
        } else if is_machine_number(&expr.copy()) {
            self.visit_machine_number(&DataMachineNumberRef::from(expr.copy()))
        } else if is_untyped_identifier(&expr.copy()) {
            self.visit_untyped_identifier(&DataUntypedIdentifierRef::from(expr.copy()))
        } else {
            unreachable!("Unknown data expression type");
        }
    }
}

pub trait PbesExpressionVisitor {
    fn visit_propositional_variable_instantiation(
        &mut self,
        _inst: &PbesPropositionalVariableInstantiationRef<'_>,
    ) -> Option<PbesExpression> {
        None
    }

    fn visit_not(&mut self, not: &PbesNotRef<'_>) -> Option<PbesExpression> {
        self.visit(&not.body());
        None
    }

    fn visit_and(&mut self, and: &PbesAndRef<'_>) -> Option<PbesExpression> {
        self.visit(&and.lhs());
        self.visit(&and.rhs());
        None
    }

    fn visit_or(&mut self, or: &PbesOrRef<'_>) -> Option<PbesExpression> {
        self.visit(&or.lhs());
        self.visit(&or.rhs());
        None
    }

    fn visit_imp(&mut self, imp: &PbesImpRef<'_>) -> Option<PbesExpression> {
        self.visit(&imp.lhs());
        self.visit(&imp.rhs());
        None
    }

    fn visit_forall(&mut self, forall: &PbesForallRef<'_>) -> Option<PbesExpression> {
        self.visit(&forall.body());
        None
    }

    fn visit_exists(&mut self, exists: &PbesExistsRef<'_>) -> Option<PbesExpression> {
        self.visit(&exists.body());
        None
    }

    fn visit(&mut self, expr: &PbesExpressionRef<'_>) -> Option<PbesExpression> {
        if is_pbes_propositional_variable_instantiation(&expr.copy()) {
            self.visit_propositional_variable_instantiation(&PbesPropositionalVariableInstantiationRef::from(
                expr.copy()
            ))
        } else if is_pbes_not(&expr.copy()) {
            self.visit_not(&PbesNotRef::from(expr.copy()))
        } else if is_pbes_and(&expr.copy()) {
            self.visit_and(&PbesAndRef::from(expr.copy()))
        } else if is_pbes_or(&expr.copy()) {
            self.visit_or(&PbesOrRef::from(expr.copy()))
        } else if is_pbes_imp(&expr.copy()) {
            self.visit_imp(&PbesImpRef::from(expr.copy()))
        } else if is_pbes_forall(&expr.copy()) {
            self.visit_forall(&PbesForallRef::from(expr.copy()))
        } else if is_pbes_exists(&expr.copy()) {
            self.visit_exists(&PbesExistsRef::from(expr.copy()))
        } else {
            unreachable!("Unknown pbes expression type");
        }
    }
}

/// Replaces data variables in the given data expression according to the
/// provided substitution function.
pub fn data_expression_replace_variables<F>(expr: &DataExpressionRef<'_>, f: &F) -> DataExpression
where
    F: Fn(&DataVariableRef<'_>) -> DataExpression,
{
    struct ReplaceVariableBuilder<'a, F> {
        apply: &'a F,
    }

    impl<'a, F> DataExpressionVisitor for ReplaceVariableBuilder<'a, F>
    where
        F: Fn(&DataVariableRef<'_>) -> DataExpression,
    {
        fn visit_variable(&mut self, var: &DataVariableRef<'_>) -> Option<DataExpression> {
            Some((*self.apply)(var))
        }
    }

    let mut builder = ReplaceVariableBuilder { apply: f };
    builder.visit(expr).expect("Replacement should return a value")
}

/// Returns all the PVIs occurring in the given PBES expression.
pub fn pbes_expression_pvi(expr: &PbesExpressionRef<'_>) -> Vec<PbesPropositionalVariableInstantiation> {
    let mut result = Vec::new();

    struct PviChecker<'a> {
        result: &'a mut Vec<PbesPropositionalVariableInstantiation>,
    }

    impl PbesExpressionVisitor for PviChecker<'_> {
        fn visit_propositional_variable_instantiation(
            &mut self,
            inst: &PbesPropositionalVariableInstantiationRef<'_>,
        ) -> Option<PbesExpression> {
            // Found a propositional variable instantiation, return true.
            self.result
                .push(PbesPropositionalVariableInstantiation::from(inst.protect()));
            None
        }
    }

    let mut checker = PviChecker { result: &mut result };

    checker.visit(expr);
    result
}
