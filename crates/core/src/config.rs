use crate::{Error, Result};
use std::{env, str::FromStr};

/// Settings read from environment variables. Every optional limit is off by default.
#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub bind: String,
    pub allow_signup: bool,
    pub cookie_secure: bool,
    pub session_days: i64,
    /// Maximum scans a workspace may start per UTC day. `None`: no limit.
    pub scan_cap_per_day: Option<u32>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_url: env::var("DATABASE_URL")
                .map_err(|_| Error::Internal("DATABASE_URL is not set".into()))?,
            bind: env::var("OPENJAM_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            allow_signup: parse("OPENJAM_ALLOW_SIGNUP")?.unwrap_or(true),
            cookie_secure: parse("OPENJAM_COOKIE_SECURE")?.unwrap_or(false),
            session_days: parse("OPENJAM_SESSION_DAYS")?.unwrap_or(30),
            scan_cap_per_day: parse("OPENJAM_SCAN_CAP_PER_DAY")?,
        })
    }
}

fn parse<T: FromStr>(name: &str) -> Result<Option<T>> {
    match env::var(name) {
        Ok(v) if !v.trim().is_empty() => v
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| Error::Internal(format!("{name} has an invalid value: {v}"))),
        _ => Ok(None),
    }
}
