//! Debug tracing functionality for the mCRL3 toolset
//! This file provides debug tracing that is only active when the merc_debug feature is enabled

/// Macro that prints debug trace information only when the merc_debug feature is enabled.
/// When enabled, this macro delegates to the standard trace! macro from the log crate.
///
/// # Examples
///
/// ```
/// debug_trace!("Processing item {}", item_id);
/// debug_trace!("Complex calculation result: {:#?}", result);
/// ```
#[macro_export]
#[cfg(feature = "merc_debug-trace")]
macro_rules! debug_trace {
    ($($arg:tt)*) => {
        {
            log::trace!($($arg)*);
        }
    };
}

#[macro_export]
#[cfg(not(feature = "merc_debug-trace"))]
macro_rules! debug_trace {
    ($($arg:tt)*) => {{
        // No-op when merc_debug is not enabled
    }};
}

/// Macro that conditionally uses items only when the merc_debug-trace feature is enabled.
/// This is useful for importing items that are only needed for debug tracing.
///
/// # Examples
///
/// ```
/// debug_use!(std::collections::HashMap);
/// debug_use!(crate::internal::debug_helper);
/// ```
#[macro_export]
#[cfg(feature = "merc_debug-trace")]
macro_rules! debug_use {
    ($($item:tt)*) => {
        use $($item)*;
    };
}

#[macro_export]
#[cfg(not(feature = "merc_debug-trace"))]
macro_rules! debug_use {
    ($($item:tt)*) => {
        // No-op when merc_debug-trace is not enabled
    };
}
