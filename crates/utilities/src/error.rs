use core::error::Error;
use core::fmt::Debug;
use core::fmt::Display;

/// The MCRL3 error type. This has a blanket [`From`] impl for any type that implements Rust's [`Error`],
/// meaning it can be used as a "catch all" error. Captures a backtrace that can be printed from this object.
pub struct MercError {
    inner: Box<InnerMCRL3Error>,
}

impl MercError {
    /// Attempts to downcast the internal error to the given type.
    pub fn downcast_ref<E: Error + 'static>(&self) -> Option<&E> {
        self.inner.error.downcast_ref::<E>()
    }
}

/// This type exists to make [`MCRL3Error`] use a "thin pointer" instead of
/// a "fat pointer", which reduces the size of our Result by a usize. This does introduce an extra indirection, but error handling is a "cold path".
/// We don't need to optimize it to that degree.
struct InnerMCRL3Error {
    /// The underlying error
    error: Box<dyn Error + Send + Sync + 'static>,
    /// A backtrace captured at creation
    backtrace: std::backtrace::Backtrace,
}

// NOTE: writing the impl this way gives us From<&str>
impl<E> From<E> for MercError
where
    Box<dyn Error + Send + Sync + 'static>: From<E>,
{
    #[cold]
    fn from(error: E) -> Self {
        MercError {
            inner: Box::new(InnerMCRL3Error {
                error: error.into(),
                backtrace: std::backtrace::Backtrace::capture(),
            }),
        }
    }
}

impl Display for MercError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "{}", self.inner.error)?;
        Ok(())
    }
}

impl Debug for MercError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "{:?}", self.inner.error)?;
        {
            let backtrace = &self.inner.backtrace;
            if let std::backtrace::BacktraceStatus::Captured = backtrace.status() {
                writeln!(f, "{backtrace}")?;
            }
        }

        Ok(())
    }
}
