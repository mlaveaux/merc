use merc_aterm::ATermRef;
use merc_aterm::Term;
use merc_data::DataExpressionRef;

use crate::DataExpressionFFI;
use crate::DataExpressionRefFFI;
use crate::DataFunctionSymbolRefFFI;

/// # Safety
///
/// See the documentation in the import module.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn data_expression_arg<'a>(
    term: &DataExpressionRefFFI<'a>,
    index: usize,
) -> DataExpressionRefFFI<'a> {
    unsafe {
        DataExpressionRefFFI::from_index(
            DataExpressionRef::from(ATermRef::from_index(term.shared()))
                .data_arg(index)
                .shared(),
        )
    }
}

/// # Safety
///
/// See the documentation in the import module.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn data_expression_symbol<'a>(term: &DataExpressionRefFFI<'a>) -> DataFunctionSymbolRefFFI<'a> {
    unsafe {
        DataFunctionSymbolRefFFI::from_index(
            DataExpressionRef::from(ATermRef::from_index(term.shared()))
                .data_function_symbol()
                .shared(),
        )
    }
}

/// # Safety
///
/// See the documentation in the import module.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn data_expression_protect(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {
    unsafe {
        let t = ATermRef::from_index(term.shared()).protect();

        let d = DataExpressionFFI::from_index(t.shared(), t.root());

        // We are now responsible for the memory of the data expression.
        std::mem::forget(t);

        d
    }
}
