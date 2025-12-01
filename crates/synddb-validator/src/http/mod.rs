//! HTTP API module for validator health checks and status

mod api;

pub use api::{create_router, AppState};
