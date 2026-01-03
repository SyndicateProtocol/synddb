//! Proof service configuration

use clap::Parser;

/// GPU-accelerated SP1 proof generation service
#[derive(Debug, Clone, Parser)]
#[command(author, version, about)]
pub struct Config {
    /// Address to bind the HTTP server
    #[arg(long, env = "BIND_ADDRESS", default_value = "0.0.0.0:8080")]
    pub bind_address: String,

    /// Enable JSON logging output
    #[arg(long, env = "LOG_JSON", default_value = "false")]
    pub log_json: bool,

    /// Google OIDC discovery URL for fetching JWKS
    #[arg(
        long,
        env = "GOOGLE_OIDC_DISCOVERY_URL",
        default_value = "https://confidentialcomputing.googleapis.com/.well-known/openid-configuration"
    )]
    pub google_oidc_discovery_url: String,

    /// JWKS cache TTL in seconds
    #[arg(long, env = "JWKS_CACHE_TTL_SECS", default_value = "3600")]
    pub jwks_cache_ttl_secs: u64,
}
