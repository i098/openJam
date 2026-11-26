//! Shared logic for the API and the worker: config, errors, auth primitives, job queue, usage ledger.

pub mod actions;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
pub mod jobs;
pub mod research;
pub mod tracking;
pub mod tracking_parse;
pub mod usage;

pub use error::{Error, Result};
