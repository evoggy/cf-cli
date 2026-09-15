use std::fmt;

/// Classified errors that map to non-zero exit codes the CLI promises to its
/// callers (humans and scripts/agents). The `exit_code` is the contract; the
/// human-readable message is best-effort and can change.
#[derive(Debug)]
pub enum CliError {
    Connection(String),
    NotFound(String),
    /// A required argument wasn't supplied and the CLI is non-interactive
    /// (or `--non-interactive` was passed). Shares exit code 30 with
    /// `InvalidValue` — both are the caller's responsibility to fix.
    MissingArg(String),
    InvalidValue(String),
    Timeout(String),
    Interrupted,
    Terminated,
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Connection(_) => 10,
            CliError::NotFound(_) => 20,
            CliError::MissingArg(_) | CliError::InvalidValue(_) => 30,
            CliError::Timeout(_) => 40,
            CliError::Interrupted => 130,
            CliError::Terminated => 143,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Connection(s) => write!(f, "connection error: {}", s),
            CliError::NotFound(s) => write!(f, "not found: {}", s),
            CliError::MissingArg(s) => write!(f, "missing argument: {}", s),
            CliError::InvalidValue(s) => write!(f, "invalid value: {}", s),
            CliError::Timeout(s) => write!(f, "timeout: {}", s),
            CliError::Interrupted => write!(f, "interrupted"),
            CliError::Terminated => write!(f, "terminated"),
        }
    }
}

impl std::error::Error for CliError {}

/// Walk the anyhow error chain and return the most specific exit code we can
/// derive. Explicit `CliError` annotations win; otherwise we map known
/// `crazyflie_lib::Error` variants to the appropriate bucket. Unclassified
/// failures return `1`.
pub fn classify_exit_code(err: &anyhow::Error) -> i32 {
    for cause in err.chain() {
        if let Some(cli) = cause.downcast_ref::<CliError>() {
            return cli.exit_code();
        }
        if let Some(cf_err) = cause.downcast_ref::<crazyflie_lib::Error>() {
            // We rely on typed variants only. `ParamError`/`LogError` carry
            // free-form strings (including missing-name errors) — the modules
            // pre-check known names and raise `CliError::NotFound` themselves
            // before these strings can bubble up here.
            match cf_err {
                crazyflie_lib::Error::VariableNotFound => return 20,
                crazyflie_lib::Error::LinkError(_)
                | crazyflie_lib::Error::Disconnected => return 10,
                crazyflie_lib::Error::InvalidArgument(_)
                | crazyflie_lib::Error::InvalidParameter(_)
                | crazyflie_lib::Error::ConversionError(_) => return 30,
                _ => {}
            }
        }
    }
    1
}
