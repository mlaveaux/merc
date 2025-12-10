/// A copy of vec![] that can be used for the [`crate::ByteCompressedVec`].
#[macro_export]
macro_rules! bytevec {
    () => {
        $crate::ByteCompressedVec::new()
    };
    ($elem:expr; $n:expr) => {
        $crate::ByteCompressedVec::from_elem($elem, $n)
    };
}
