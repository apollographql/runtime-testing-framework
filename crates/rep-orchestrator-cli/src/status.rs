/// Log an informational message and post a status update to the orchestrator.
///
/// Defined as a macro so that [tracing::info!] captures the caller's source location and format strings can be supported.
#[macro_export]
macro_rules! info_status {
    ($ctx:expr, $status:expr, $($arg:tt)+) => {{
        use $crate::orchestrator::Client as _;

        let message = format!($($arg)+);
        tracing::info!(message);
        $ctx.orchestrator_client()
            .update_status($status, None, Some(message))
            .await
            .map_err($crate::error::Error::from)
    }};
}
