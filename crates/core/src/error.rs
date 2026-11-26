use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// One error type for the whole workspace. `code` and `status` map to the `/v1` error body.
#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("missing, malformed or revoked credentials")]
    Unauthorized,
    #[error("you cannot do this")]
    Forbidden,
    #[error("API keys cannot do this; sign in with the dashboard")]
    ForbiddenForApiKey,
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(String),
    #[error(
        "scan cap of {cap} a day reached; raise OPENJAM_SCAN_CAP_PER_DAY or wait for 00:00 UTC"
    )]
    CapReached { cap: u32 },
    #[error("not implemented in this build")]
    NotImplemented,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("{0}")]
    Internal(String),
}

impl Error {
    pub fn status(&self) -> u16 {
        match self {
            Self::InvalidRequest(_) => 400,
            Self::Unauthorized => 401,
            Self::Forbidden | Self::ForbiddenForApiKey => 403,
            Self::NotFound(_) => 404,
            Self::Conflict(_) => 409,
            Self::CapReached { .. } => 429,
            Self::NotImplemented => 501,
            Self::Database(_) | Self::Migrate(_) | Self::Internal(_) => 500,
        }
    }

    /// Stable machine-readable code, the same set as the `/v1` API.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "invalid_request",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::ForbiddenForApiKey => "forbidden_for_api_key",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::CapReached { .. } => "cap_reached",
            Self::NotImplemented => "not_implemented",
            Self::Database(_) | Self::Migrate(_) | Self::Internal(_) => "internal_error",
        }
    }

    /// Message for the client. Internal causes stay in the log.
    pub fn public_message(&self) -> String {
        match self {
            Self::Database(_) | Self::Migrate(_) | Self::Internal(_) => {
                "internal error; quote the requestId".into()
            }
            other => other.to_string(),
        }
    }
}
