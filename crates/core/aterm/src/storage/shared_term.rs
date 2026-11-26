use std::alloc::Layout;
use std::alloc::LayoutError;
use std::fmt;
use std::hash::Hash;
use std::mem::offset_of;
use std::ptr::NonNull;
use std::ptr::slice_from_raw_parts_mut;

use equivalent::Equivalent;
use merc_unsafety::Erasable;
use merc_unsafety::ErasedPtr;
use merc_unsafety::SliceDst;
use merc_unsafety::StablePointer;
use merc_unsafety::repr_c;
use merc_utilities::AtomicTaggedPtr;

use crate::ATermRef;
use crate::Symb;
use crate::SymbolRef;
use crate::Term;
use crate::Transmutable;
use crate::storage::SymbolHeader;

/// The underlying type of terms that are maximally shared.
///
/// # Details
///
/// Uses a C representation and is a dynamically sized type for compact memory
/// usage, implementing [SliceDst] and [Erasable]. This allows us to avoid
/// storing the length and capacity of an underlying vector. As such this is
/// even more compact than `smallvec`. Arguments are stored as [ATermRef] slices.
#[repr(C)]
pub struct SharedTerm {
    symbol: TermSymbol,
    arguments: [ATermRef<'static>],
}

/// The function symbol of a stored term, whose lowest address bit marks the term as reachable
/// during garbage collection.
///
/// The collector sets and clears the mark while it holds the term pool exclusively, but other
/// threads may read the symbol meanwhile, which is why it is atomic and reads ignore the mark.
#[repr(C)]
pub(crate) struct TermSymbol {
    /// Points to the header of the symbol rather than the symbol itself, since the symbol is a
    /// dynamically sized type whose wide pointer cannot be stored atomically.
    ptr: AtomicTaggedPtr<SymbolHeader>,

    /// Holds the symbol's debug reference counter, see [`StablePointer`].
    #[cfg(debug_assertions)]
    reference: SymbolRef<'static>,
}

impl TermSymbol {
    /// Stores the given symbol, which the term keeps alive because marking a term marks its symbol.
    pub(crate) fn new(symbol: &SymbolRef<'_>) -> Self {
        Self {
            ptr: AtomicTaggedPtr::new(symbol.shared().erased().cast()),
            // SAFETY: the symbol lives as long as the term, see above.
            #[cfg(debug_assertions)]
            reference: unsafe { SymbolRef::from_index(symbol.shared()) },
        }
    }

    /// Returns the symbol.
    pub(crate) fn get(&self) -> SymbolRef<'_> {
        // SAFETY: the stored pointer was not null, and masking the mark keeps it so.
        let ptr: ErasedPtr = unsafe { NonNull::new_unchecked(self.ptr.load()) }.cast();

        // SAFETY: `ptr` is the erased pointer of the symbol that this term keeps alive, see `new`,
        // which is also the symbol of `reference`.
        #[cfg(debug_assertions)]
        let index = unsafe { StablePointer::from_related_erased(ptr, self.reference.shared()) };
        #[cfg(not(debug_assertions))]
        let index = unsafe { StablePointer::from_erased(ptr) };

        // SAFETY: the returned reference borrows `self`, which keeps the symbol alive.
        unsafe { SymbolRef::from_index(&index) }
    }

    /// Marks the term and returns true iff it was not marked before.
    pub(crate) fn mark(&self) -> bool {
        !self.ptr.set_tag()
    }

    /// Returns true iff the term is marked.
    pub(crate) fn is_marked(&self) -> bool {
        self.ptr.is_tagged()
    }

    /// Removes the mark of the term.
    pub(crate) fn unmark(&self) {
        self.ptr.clear_tag()
    }
}

impl PartialEq for TermSymbol {
    fn eq(&self, other: &Self) -> bool {
        self.ptr.load() == other.ptr.load()
    }
}

impl Eq for TermSymbol {}

impl Hash for TermSymbol {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Must equal the hash of the symbol itself, which lookups use.
        self.get().hash(state);
    }
}

impl fmt::Debug for TermSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.get())
    }
}

impl PartialEq for SharedTerm {
    fn eq(&self, other: &Self) -> bool {
        self.symbol == other.symbol && self.arguments == other.arguments
    }
}

impl Eq for SharedTerm {}

/// Note that the length is stored in the symbol's arity
unsafe impl SliceDst for SharedTerm {
    fn layout_for(len: usize) -> Result<Layout, LayoutError> {
        let header_layout = Layout::new::<TermSymbol>();
        let slice_layout = Layout::array::<ATermRef<'static>>(len)?;

        repr_c(&[header_layout, slice_layout])
    }

    fn retype(ptr: std::ptr::NonNull<[()]>) -> NonNull<Self> {
        unsafe { NonNull::new_unchecked(ptr.as_ptr() as *mut _) }
    }

    fn length(&self) -> usize {
        self.symbol().arity()
    }
}

unsafe impl Erasable for SharedTerm {
    fn erase(this: NonNull<Self>) -> ErasedPtr {
        this.cast()
    }

    unsafe fn unerase(this: ErasedPtr) -> NonNull<Self> {
        unsafe {
            // Every term starts with its symbol, whose arity is the number of arguments.
            let len = (*this.as_ptr().cast::<TermSymbol>()).get().arity();

            let raw = NonNull::new_unchecked(slice_from_raw_parts_mut(this.as_ptr().cast(), len));
            Self::retype(raw)
        }
    }
}

impl fmt::Debug for SharedTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SharedTerm {{ symbol: {:?}, arguments: {:?} }}",
            self.symbol,
            self.arguments()
        )
    }
}

/// Offset of the arguments, which follow the [TermSymbol] header in the `#[repr(C)]` layout.
const ARGUMENTS_OFFSET: usize = size_of::<TermSymbol>().next_multiple_of(align_of::<ATermRef<'static>>());

impl SharedTerm {
    /// Returns the symbol of the term at `ptr` by reading its header, without
    /// reconstructing the wide pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a live [SharedTerm] that outlives `'a`.
    pub(crate) unsafe fn symbol_of<'a>(ptr: ErasedPtr) -> SymbolRef<'a> {
        // SAFETY: the caller guarantees that `ptr` points to a live term, which starts with its symbol.
        unsafe { ptr.cast::<TermSymbol>().as_ref() }.get()
    }

    /// Returns the argument at `index` of the term at `ptr`, without
    /// reconstructing the wide pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a live [SharedTerm] that outlives `'a`, and `index`
    /// must be less than its arity.
    pub(crate) unsafe fn argument_of<'a>(ptr: ErasedPtr, index: usize) -> &'a ATermRef<'static> {
        // SAFETY: the caller guarantees that `index` is in bounds of the arguments of a live term,
        // which start at `ARGUMENTS_OFFSET`.
        unsafe {
            ptr.byte_add(ARGUMENTS_OFFSET)
                .cast::<ATermRef<'static>>()
                .add(index)
                .as_ref()
        }
    }

    /// Returns the symbol of the term.
    pub fn symbol(&self) -> SymbolRef<'_> {
        self.symbol.get()
    }

    /// Marks the term as reachable and returns true iff it was not marked before.
    pub(crate) fn mark(&self) -> bool {
        self.symbol.mark()
    }

    /// Returns true iff the term is marked as reachable.
    pub(crate) fn is_marked(&self) -> bool {
        self.symbol.is_marked()
    }

    /// Removes the reachability mark of the term.
    pub(crate) fn unmark(&self) {
        self.symbol.unmark()
    }

    /// Returns the arguments of the term.
    pub fn arguments(&self) -> &[ATermRef<'_>] {
        // SAFETY: The returned lifetime is bound to the borrow of `self` by the signature.
        unsafe { self.arguments.transmute_lifetime() }
    }

    /// Returns a unique index for this shared term.
    pub fn index(&self) -> usize {
        self as *const Self as *const u8 as usize
    }

    /// Returns the length for a [SharedTermLookup]
    pub(crate) fn length_for(object: &SharedTermLookup) -> usize {
        object.arguments.len()
    }

    /// Constructs an uninitialised ptr from a [SharedTermLookup]
    pub(crate) unsafe fn construct(ptr: *mut SharedTerm, object: &SharedTermLookup) {
        unsafe {
            ptr.cast::<TermSymbol>().write(TermSymbol::new(&object.symbol));

            for (index, argument) in object.arguments.iter().enumerate() {
                ptr.byte_add(ARGUMENTS_OFFSET)
                    .cast::<ATermRef<'static>>()
                    .add(index)
                    .write(ATermRef::from_index(argument.shared()));
            }
        }
    }
}

impl Hash for SharedTerm {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.symbol.hash(state);
        self.arguments.hash(state);
    }
}

/// A cheap reference to the elements of a [SharedTerm] that can be used for
/// lookup of terms without allocating.
pub(crate) struct SharedTermLookup<'a> {
    pub(crate) symbol: SymbolRef<'a>,
    pub(crate) arguments: &'a [ATermRef<'a>],
}

impl Equivalent<SharedTerm> for SharedTermLookup<'_> {
    fn equivalent(&self, other: &SharedTerm) -> bool {
        self.symbol == other.symbol() && self.arguments == &other.arguments
    }
}

/// This Hash implement must be the same as for [SharedTerm]
impl Hash for SharedTermLookup<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.symbol.hash(state);
        self.arguments.hash(state);
    }
}

// `symbol` must be at offset 0 in all term representations so that any pointer to a term.
const _: () = assert!(offset_of!(SharedTerm, symbol) == 0);
