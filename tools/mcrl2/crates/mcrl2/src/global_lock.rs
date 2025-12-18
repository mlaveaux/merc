use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::MutexGuard;

pub type GlobalLockGuard = MutexGuard<'static, ()>;

/// A global lock for non thread safe FFI functions.
pub fn lock_global() -> GlobalLockGuard {
    GLOBAL_MUTEX.lock().expect("Failed to lock GLOBAL_MUTEX")
}

/// This is the global mutex used to guard non thread safe FFI functions.
pub(crate) static GLOBAL_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));