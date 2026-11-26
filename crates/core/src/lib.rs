//! Shared logic for the API and the worker: config, errors, auth primitives, job queue, usage ledger.

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod jobs;
pub mod tracking;
pub mod tracking_parse;
pub mod usage;

pub use error::{Error, Result};
