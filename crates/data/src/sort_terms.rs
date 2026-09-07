use std::fmt;
use std::ops::Deref;

use delegate::delegate;

use merc_aterm::ATerm;
use merc_aterm::ATermArgs;
use merc_aterm::ATermIndex;
use merc_aterm::ATermList;
use merc_aterm::ATermRef;
use merc_aterm::ATermString;
use merc_aterm::Markable;
use merc_aterm::Symb;
use merc_aterm::SymbolRef;
use merc_aterm::Term;
use merc_aterm::TermIterator;
use merc_aterm::Transmutable;
use merc_aterm::storage::Marker;
use merc_macros::merc_derive_terms;
use merc_macros::merc_ignore;
use merc_macros::merc_term;

use crate::DATA_SYMBOLS;
use crate::is_basic_sort;
use crate::is_container_sort;
use crate::is_function_sort;
use crate::is_sort_alias;
use crate::is_sort_expression;

/// This module is only used internally to run the proc macro.
#[merc_derive_terms]
mod inner {
    use super::*;

    #[merc_term(is_sort_expression)]
    pub struct SortExpression {
        term: ATerm,
    }

    impl SortExpression {
        /// Returns the name of the sort.
        pub fn name(&self) -> &str {
            self.term.arg(0).get_head_symbol().name()
        }

        /// Creates a basic sort with the unknown value.
        pub fn unknown_sort() -> SortExpression {
            DATA_SYMBOLS.with_borrow(|ds| SortExpression {
                term: ATerm::with_args(ds.basic_sort_symbol.deref(), &[ATermString::new("@no_value@")]).protect(),
            })
        }
    }

    impl fmt::Display for SortExpression {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            // `name()` only makes sense for a basic sort: it reads `arg(0)` as
            // an identifier, which for a `SortArrow`/`SortCons` term is really
            // the domain list/container-kind tag, not a name.
            if is_basic_sort(&self.term) {
                write!(f, "{}", self.name())
            } else if is_function_sort(&self.term) {
                write!(f, "{}", SortArrowRef::from(self.term.copy()))
            } else if is_container_sort(&self.term) {
                write!(f, "{}", SortConsRef::from(self.term.copy()))
            } else {
                write!(f, "{}", self.term)
            }
        }
    }

    #[merc_term(is_basic_sort)]
    pub struct BasicSort {
        term: ATerm,
    }

    impl BasicSort {
        /// Creates a basic sort with the given name (`Bool`, `Pos`, `Nat`, `Int`,
        /// `Real`, or a declared sort's name).
        #[merc_ignore]
        pub fn new<N: Into<ATermString>>(name: N) -> BasicSort {
            DATA_SYMBOLS.with_borrow(|ds| BasicSort {
                term: ATerm::with_args(ds.basic_sort_symbol.deref(), &[name.into()]).protect(),
            })
        }

        /// Returns the name of the sort.
        pub fn name(&self) -> &str {
            self.term.arg(0).get_head_symbol().name()
        }
    }

    impl fmt::Display for BasicSort {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.name())
        }
    }

    /// A function sort `domain_0 # ... # domain_n -> codomain` (mCRL2's `SortArrow`).
    #[merc_term(is_function_sort)]
    pub struct SortArrow {
        term: ATerm,
    }

    impl SortArrow {
        /// Builds the function sort `domain -> codomain`, `domain` a proper
        /// aterm list).
        #[merc_ignore]
        pub fn new(domain: &[SortExpression], codomain: SortExpression) -> SortArrow {
            DATA_SYMBOLS.with_borrow(|ds| {
                let domain_list: ATermList<SortExpression> = ATermList::from_double_iter(domain.iter().cloned());
                let args: [ATerm; 2] = [domain_list.into(), codomain.into()];
                SortArrow {
                    term: ATerm::with_args(ds.function_sort_symbol.deref(), &args).protect(),
                }
            })
        }

        /// Returns the domain sorts, in declaration order.
        pub fn domain(&self) -> ATermList<SortExpression> {
            self.term.arg(0).into()
        }

        /// Returns the codomain (result) sort.
        pub fn codomain(&self) -> SortExpressionRef<'_> {
            self.term.arg(1).into()
        }
    }

    impl fmt::Display for SortArrow {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{} -> {}", self.domain(), self.codomain())
        }
    }

    /// The kind of a container sort, the first argument of a `SortCons` rather
    /// than encoded in its symbol name.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
    pub enum ContainerSortKind {
        List,
        Set,
        Bag,
        FSet,
        FBag,
    }

    #[merc_ignore]
    impl fmt::Display for ContainerSortKind {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(match self {
                ContainerSortKind::List => "List",
                ContainerSortKind::Set => "Set",
                ContainerSortKind::Bag => "Bag",
                ContainerSortKind::FSet => "FSet",
                ContainerSortKind::FBag => "FBag",
            })
        }
    }

    #[merc_ignore]
    impl ContainerSortKind {
        fn to_term(self) -> ATerm {
            DATA_SYMBOLS.with_borrow(|ds| {
                let symbol = match self {
                    ContainerSortKind::List => ds.list_container_symbol.deref(),
                    ContainerSortKind::Set => ds.set_container_symbol.deref(),
                    ContainerSortKind::Bag => ds.bag_container_symbol.deref(),
                    ContainerSortKind::FSet => ds.fset_container_symbol.deref(),
                    ContainerSortKind::FBag => ds.fbag_container_symbol.deref(),
                };
                ATerm::constant(symbol)
            })
        }

        /// The inverse of [`Self::to_term`]: reads the kind back from the
        /// leaf term stored as a `SortCons`'s first argument.
        fn from_term<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> ContainerSortKind {
            DATA_SYMBOLS.with_borrow(|ds| {
                let symbol = term.get_head_symbol();
                if symbol == ds.list_container_symbol.copy() {
                    ContainerSortKind::List
                } else if symbol == ds.set_container_symbol.copy() {
                    ContainerSortKind::Set
                } else if symbol == ds.bag_container_symbol.copy() {
                    ContainerSortKind::Bag
                } else if symbol == ds.fset_container_symbol.copy() {
                    ContainerSortKind::FSet
                } else if symbol == ds.fbag_container_symbol.copy() {
                    ContainerSortKind::FBag
                } else {
                    unreachable!("not a container-kind term")
                }
            })
        }
    }

    /// A container sort `kind(element)`, e.g. `List(Nat)`.
    #[merc_term(is_container_sort)]
    pub struct SortCons {
        term: ATerm,
    }

    impl SortCons {
        #[merc_ignore]
        pub fn new(kind: ContainerSortKind, element: SortExpression) -> SortCons {
            DATA_SYMBOLS.with_borrow(|ds| {
                let args: [ATerm; 2] = [kind.to_term(), element.into()];
                SortCons {
                    term: ATerm::with_args(ds.container_sort_symbol.deref(), &args).protect(),
                }
            })
        }

        /// Returns the sort of the container's elements.
        pub fn element_sort(&self) -> SortExpressionRef<'_> {
            self.term.arg(1).into()
        }

        /// Returns the kind of container (`List`, `Set`, `Bag`, `FSet`, `FBag`).
        pub fn kind(&self) -> ContainerSortKind {
            ContainerSortKind::from_term(&self.term.arg(0))
        }
    }

    impl fmt::Display for SortCons {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}({})", self.kind(), self.element_sort())
        }
    }

    /// A sort alias `name = reference` (mCRL2's `SortRef`), e.g. `sort D = List(Nat);`.
    #[merc_term(is_sort_alias)]
    pub struct SortAlias {
        term: ATerm,
    }

    impl SortAlias {
        /// Builds the alias `name = reference`.
        #[merc_ignore]
        pub fn new(name: BasicSort, reference: SortExpression) -> SortAlias {
            DATA_SYMBOLS.with_borrow(|ds| {
                let args: [ATerm; 2] = [name.into(), reference.into()];
                SortAlias {
                    term: ATerm::with_args(ds.sort_alias_symbol.deref(), &args).protect(),
                }
            })
        }

        /// Returns the name being aliased.
        pub fn name(&self) -> BasicSortRef<'_> {
            self.term.arg(0).into()
        }

        /// Returns the sort the name refers to.
        pub fn reference(&self) -> SortExpressionRef<'_> {
            self.term.arg(1).into()
        }
    }

    impl fmt::Display for SortAlias {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{} = {}", self.name(), self.reference())
        }
    }

    #[merc_ignore]
    impl From<BasicSort> for SortExpression {
        fn from(value: BasicSort) -> Self {
            value.term.into()
        }
    }

    #[merc_ignore]
    impl From<SortArrow> for SortExpression {
        fn from(value: SortArrow) -> Self {
            value.term.into()
        }
    }

    #[merc_ignore]
    impl From<SortCons> for SortExpression {
        fn from(value: SortCons) -> Self {
            value.term.into()
        }
    }
}

pub use inner::BasicSort;
pub use inner::BasicSortRef;
pub use inner::ContainerSortKind;
pub use inner::SortAlias;
pub use inner::SortArrow;
pub use inner::SortArrowRef;
pub use inner::SortCons;
pub use inner::SortConsRef;
pub use inner::SortExpression;
pub use inner::SortExpressionRef;

#[cfg(test)]
mod tests {
    use super::BasicSort;
    use super::ContainerSortKind;
    use super::SortAlias;
    use super::SortArrow;
    use super::SortCons;
    use super::SortExpression;
    use super::is_container_sort;
    use super::is_function_sort;
    use super::is_sort_expression;
    use crate::is_basic_sort;
    use crate::is_sort_alias;

    fn basic(name: &str) -> SortExpression {
        BasicSort::new(name).into()
    }

    #[test]
    fn test_unknown_sort() {
        let sort = SortExpression::unknown_sort();
        assert_eq!(sort.name(), "@no_value@");
        assert_eq!(format!("{sort}"), "@no_value@");
        assert!(is_sort_expression(&sort));
    }

    #[test]
    fn test_sort_arrow() {
        let domain = [basic("Nat"), basic("Bool")];
        let arrow = SortArrow::new(&domain, basic("Bool"));

        assert!(is_function_sort(&arrow));
        assert!(!is_basic_sort(&arrow));
        assert_eq!(arrow.domain().to_vec(), domain);
        assert_eq!(arrow.codomain().protect(), basic("Bool"));

        let sort: SortExpression = arrow.into();
        assert!(is_sort_expression(&sort));
    }

    #[test]
    fn test_sort_cons() {
        let list_nat = SortCons::new(ContainerSortKind::List, basic("Nat"));

        assert!(is_container_sort(&list_nat));
        assert_eq!(list_nat.element_sort().protect(), basic("Nat"));
        assert_eq!(list_nat.kind(), ContainerSortKind::List);
        assert_eq!(format!("{list_nat}"), "List(Nat)");

        let sort: SortExpression = list_nat.into();
        assert!(is_sort_expression(&sort));
    }

    #[test]
    fn test_sort_cons_kind_round_trips_for_every_variant() {
        for (kind, keyword) in [
            (ContainerSortKind::List, "List"),
            (ContainerSortKind::Set, "Set"),
            (ContainerSortKind::Bag, "Bag"),
            (ContainerSortKind::FSet, "FSet"),
            (ContainerSortKind::FBag, "FBag"),
        ] {
            let cons = SortCons::new(kind, basic("Nat"));
            assert_eq!(cons.kind(), kind);
            assert_eq!(format!("{cons}"), format!("{keyword}(Nat)"));
        }
    }

    #[test]
    fn test_sort_alias() {
        let alias = SortAlias::new(BasicSort::new("D"), basic("Nat"));

        assert!(is_sort_alias(&alias));
        assert_eq!(alias.name().protect(), BasicSort::new("D"));
        assert_eq!(alias.reference().protect(), basic("Nat"));
        assert_eq!(format!("{alias}"), "D = Nat");
    }
}
