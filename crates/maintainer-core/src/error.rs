/// Top-level application error type.
///
/// Wraps subsystem failures with enough context for user-facing messages
/// without exposing raw command output or internal paths.
#[derive(Debug, thiserror::Error)]
pub enum MaintainerError {
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
    #[error("invalid data from {source_name}: {message}")]
    InvalidData {
        source_name: &'static str,
        message: String,
    },
    #[error("operation cancelled")]
    Cancelled,
    #[error("command failed: {message}")]
    CommandFailed { message: String },
    #[error("{context}: {message}")]
    Internal { context: String, message: String },
}

impl MaintainerError {
    /// Wrap an I/O error with contextual information.
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// Report invalid or unparseable data from a named source.
    pub fn invalid_data(source_name: &'static str, message: impl Into<String>) -> Self {
        Self::InvalidData {
            source_name,
            message: message.into(),
        }
    }
}

/// Error type for `CommandSpec` construction failures.
#[derive(Debug, thiserror::Error)]
pub enum CommandSpecError {
    #[error("forbidden program: {0}")]
    ForbiddenProgram(String),
    #[error("program must be an absolute path or in the allowlist: {0}")]
    RelativePath(String),
    #[error("empty program name")]
    EmptyProgram,
}

/// Result alias using our top-level error.
pub type Result<T> = std::result::Result<T, MaintainerError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_preserves_context() {
        let err = MaintainerError::io(
            "reading /proc/stat",
            std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
        );
        let msg = err.to_string();
        assert!(msg.contains("reading /proc/stat"));
        assert!(msg.contains("file not found"));
    }

    #[test]
    fn invalid_data_includes_source_name() {
        let err = MaintainerError::invalid_data("sensors", "unexpected format");
        let msg = err.to_string();
        assert!(msg.contains("sensors"));
        assert!(msg.contains("unexpected format"));
    }

    #[test]
    fn cancelled_has_clear_message() {
        let err = MaintainerError::Cancelled;
        assert_eq!(err.to_string(), "operation cancelled");
    }

    #[test]
    fn command_spec_error_forbidden_program_display() {
        let err = CommandSpecError::ForbiddenProgram("bash".into());
        assert!(err.to_string().contains("bash"));
    }
}
