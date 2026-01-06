use mcrl2_macros::mcrl2_derive_terms;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_expression_to_string;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_is_not;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_is_pbes_expression;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_is_propositional_variable_instantiation;

use crate::ATermRef;

/// Checks if this term is a data variable.
pub fn is_pbes_expression(term: &ATermRef<'_>) -> bool {
    mcrl2_pbes_is_pbes_expression(term.get())
}

pub fn is_pbes_propositional_variable_instantiation(term: &ATermRef<'_>) -> bool {
    mcrl2_pbes_is_propositional_variable_instantiation(term.get())
}

pub fn is_pbes_not(term: &ATermRef<'_>) -> bool {
    mcrl2_pbes_is_not(term.get())
}

pub fn is_pbes_and(term: &ATermRef<'_>) -> bool {
    mcrl2_sys::pbes::ffi::mcrl2_pbes_is_and(term.get())
}

pub fn is_pbes_or(term: &ATermRef<'_>) -> bool {
    mcrl2_sys::pbes::ffi::mcrl2_pbes_is_or(term.get())
}

pub fn is_pbes_imp(term: &ATermRef<'_>) -> bool {
    mcrl2_sys::pbes::ffi::mcrl2_pbes_is_imp(term.get())
}

pub fn is_pbes_forall(term: &ATermRef<'_>) -> bool {
    mcrl2_sys::pbes::ffi::mcrl2_pbes_is_forall(term.get())
}

pub fn is_pbes_exists(term: &ATermRef<'_>) -> bool {
    mcrl2_sys::pbes::ffi::mcrl2_pbes_is_exists(term.get())
}

// This module is only used internally to run the proc macro.
#[mcrl2_derive_terms]
mod inner {
    use super::*;

    use std::fmt;

    use mcrl2_macros::mcrl2_term;

    use crate::ATerm;
    use crate::ATermRef;
    use crate::Markable;
    use crate::Todo;
    use crate::is_pbes_expression;

    /// mcrl2::pbes_system::pbes_expression
    #[mcrl2_term(is_pbes_expression)]
    pub struct PbesExpression {
        term: ATerm,
    }

    impl fmt::Display for PbesExpression {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", mcrl2_pbes_expression_to_string(self.term.get()))
        }
    }

    /// Represents a mcrl2::pbes_system::propositional_variable_instantiation
    #[mcrl2_term(is_pbes_propositional_variable_instantiation)]
    pub struct PbesPropositionalVariableInstantiation {
        term: ATerm,
    }

    /// Represents a mcrl2::pbes_system::not
    #[mcrl2_term(is_pbes_not)]
    pub struct PbesNot {
        term: ATerm,
    }

    /// Represents a mcrl2::pbes_system::and_
    #[mcrl2_term(is_pbes_and)]
    pub struct PbesAnd {
        term: ATerm,
    }

    /// Represents a mcrl2::pbes_system::or_
    #[mcrl2_term(is_pbes_or)]
    pub struct PbesOr {
        term: ATerm,
    }

    /// Represents a mcrl2::pbes_system::imp
    #[mcrl2_term(is_pbes_imp)]
    pub struct PbesImp {
        term: ATerm,
    }

    /// Represents a mcrl2::pbes_system::forall
    #[mcrl2_term(is_pbes_forall)]
    pub struct PbesForall {
        term: ATerm,
    }

    /// Represents a mcrl2::pbes_system::exists
    #[mcrl2_term(is_pbes_exists)]
    pub struct PbesExists {
        term: ATerm,
    }
}

pub use inner::*;

impl From<PbesPropositionalVariableInstantiation> for PbesExpression {
    fn from(inst: PbesPropositionalVariableInstantiation) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesNot> for PbesExpression {
    fn from(inst: PbesNot) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesAnd> for PbesExpression {
    fn from(inst: PbesAnd) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesOr> for PbesExpression {
    fn from(inst: PbesOr) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesImp> for PbesExpression {
    fn from(inst: PbesImp) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesForall> for PbesExpression {
    fn from(inst: PbesForall) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesExists> for PbesExpression {
    fn from(inst: PbesExists) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesPropositionalVariableInstantiationRef<'a>> for PbesExpressionRef<'a> {
    fn from(inst: PbesPropositionalVariableInstantiationRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesNotRef<'a>> for PbesExpressionRef<'a> {
    fn from(inst: PbesNotRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesAndRef<'a>> for PbesExpressionRef<'a> {
    fn from(inst: PbesAndRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesOrRef<'a>> for PbesExpressionRef<'a> {
    fn from(inst: PbesOrRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesImpRef<'a>> for PbesExpressionRef<'a> {
    fn from(inst: PbesImpRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesForallRef<'a>> for PbesExpressionRef<'a> {
    fn from(inst: PbesForallRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesExistsRef<'a>> for PbesExpressionRef<'a> {
    fn from(inst: PbesExistsRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesExpression> for PbesPropositionalVariableInstantiation {
    fn from(inst: PbesExpression) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesExpression> for PbesNot {
    fn from(inst: PbesExpression) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesExpression> for PbesAnd {
    fn from(inst: PbesExpression) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesExpression> for PbesOr {
    fn from(inst: PbesExpression) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesExpression> for PbesImp {
    fn from(inst: PbesExpression) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesExpression> for PbesForall {
    fn from(inst: PbesExpression) -> Self {
        Self::new(inst.into())
    }
}

impl From<PbesExpression> for PbesExists {
    fn from(inst: PbesExpression) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesExpressionRef<'a>> for PbesPropositionalVariableInstantiationRef<'a> {
    fn from(inst: PbesExpressionRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesExpressionRef<'a>> for PbesNotRef<'a> {
    fn from(inst: PbesExpressionRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesExpressionRef<'a>> for PbesAndRef<'a> {
    fn from(inst: PbesExpressionRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesExpressionRef<'a>> for PbesOrRef<'a> {
    fn from(inst: PbesExpressionRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesExpressionRef<'a>> for PbesImpRef<'a> {
    fn from(inst: PbesExpressionRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesExpressionRef<'a>> for PbesForallRef<'a> {
    fn from(inst: PbesExpressionRef<'a>) -> Self {
        Self::new(inst.into())
    }
}

impl<'a> From<PbesExpressionRef<'a>> for PbesExistsRef<'a> {
    fn from(inst: PbesExpressionRef<'a>) -> Self {
        Self::new(inst.into())
    }
}
