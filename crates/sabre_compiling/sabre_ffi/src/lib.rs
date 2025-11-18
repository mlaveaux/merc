use std::marker::PhantomData;

use merc_aterm::ATermIndex;
use merc_utilities::ProtectionIndex;

#[cfg(feature = "import")]
mod import;
#[cfg(feature = "import")]
pub use import::*;

#[cfg(not(feature = "import"))]
mod export;
#[cfg(not(feature = "import"))]
pub use export::*;

#[repr(C)]
pub struct DataExpressionFFI {
    index: ATermIndex,
    root: ProtectionIndex,
}

impl DataExpressionFFI {
    /// Creates a new data expression from an index and a root.
    ///
    /// # Safety
    ///
    /// The index must be a valid index of a data expression, that is valid for this lifetime.
    pub unsafe fn from_index(index: &ATermIndex, root: ProtectionIndex) -> Self {
        Self {
            index: index.copy(),
            root,
        }
    }

    /// # Safety
    ///
    /// The index must be a valid index of a data expression, that is valid for this lifetime.
    pub fn copy(&self) -> DataExpressionRefFFI<'_> {
        unsafe { DataExpressionRefFFI::from_index(&self.index) }
    }

    /// Returns the index of the data expression.
    pub fn index(&self) -> &ATermIndex {
        &self.index
    }
}

#[repr(transparent)]
pub struct DataExpressionRefFFI<'a> {
    index: ATermIndex,
    _marker: PhantomData<&'a ()>,
}

impl DataExpressionRefFFI<'_> {
    /// # Safety
    /// The index must be a valid index of a data expression, that is valid for this lifetime.
    pub unsafe fn from_index(index: &ATermIndex) -> Self {
        Self {
            index: index.copy(),
            _marker: PhantomData,
        }
    }

    /// Returns the index of the data expression.
    pub fn shared(&self) -> &ATermIndex {
        &self.index
    }
}

impl<'a> DataExpressionRefFFI<'a> {
    /// Returns the data function symbol of the data expression.
    pub fn data_function_symbol(&self) -> DataFunctionSymbolRefFFI<'a> {
        unsafe { data_expression_symbol(self) }
    }

    /// Returns the argument of the data expression at the given index.
    pub fn data_arg(&self, index: usize) -> DataExpressionRefFFI<'a> {
        unsafe { data_expression_arg(self, index) }
    }

    /// Returns a copy of the data expression.
    pub fn copy(&self) -> DataExpressionRefFFI<'a> {
        unsafe { DataExpressionRefFFI::from_index(&self.index) }
    }

    /// Protects the data expression, preventing it from being garbage collected.
    pub fn protect(&self) -> DataExpressionFFI {
        unsafe { data_expression_protect(self) }
    }
}

#[repr(C)]
pub struct DataFunctionSymbolRefFFI<'a> {
    index: ATermIndex,
    _marker: PhantomData<&'a ()>,
}

impl DataFunctionSymbolRefFFI<'_> {
    /// # Safety
    /// The index must be a valid index of a data function symbol, that is valid for this lifetime.
    unsafe fn from_index(index: &ATermIndex) -> Self {
        Self {
            index: index.copy(),
            _marker: PhantomData,
        }
    }
}

impl DataFunctionSymbolRefFFI<'_> {
    /// Returns the operation id of the data function symbol.
    pub fn operation_id(&self) -> usize {
        self.index.index()
    }
}
