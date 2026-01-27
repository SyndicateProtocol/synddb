//! HTTP REST API for the prediction market
//!
//! Provides a JSON API for all market operations, demonstrating how to integrate
//! `SyndDB` with a typical web service.
//!
//! # Threading Considerations
//!
//! `rusqlite::Connection` is not `Sync` or `Send`, so we can't directly share
//! a `PredictionMarket` across async tasks. This example uses a simple approach:
//! store configuration in state and create a connection per request.
//!
//! Production apps might use:
//! - A connection pool (r2d2, deadpool-sqlite)
//! - `tokio::task::spawn_blocking` with a connection per task
//! - A dedicated database thread with channels

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::{
    app::{Account, PredictionMarket},
    bridge::Withdrawal,
    market::Market,
    trading::{Position, Trade},
};

/// Configuration for creating database connections
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub db_path: String,
    pub sequencer_url: Option<String>,
}

/// Shared application state for HTTP handlers
pub type AppState = Arc<AppConfig>;

impl AppConfig {
    /// Create a new `PredictionMarket` connection
    ///
    /// This creates a fresh connection for each request, which is simple but
    /// not optimal for high throughput. Production apps should use connection
    /// pooling.
    fn connect(&self) -> anyhow::Result<PredictionMarket> {
        PredictionMarket::new(&self.db_path, self.sequencer_url.as_deref())
    }
}

/// Create the HTTP router with all endpoints
pub fn create_router(db_path: String, sequencer_url: Option<String>) -> Router {
    let state: AppState = Arc::new(AppConfig {
        db_path,
        sequencer_url,
    });

    Router::new()
        // Account endpoints
        .route("/accounts", post(create_account))
        .route("/accounts", get(list_accounts))
        .route("/accounts/{id}", get(get_account))
        .route("/accounts/{id}/positions", get(get_positions))
        // Market endpoints
        .route("/markets", post(create_market))
        .route("/markets", get(list_markets))
        .route("/markets/{id}", get(get_market))
        .route("/markets/{id}/buy", post(buy_shares))
        .route("/markets/{id}/sell", post(sell_shares))
        .route("/markets/{id}/resolve", post(resolve_market))
        // Bridge endpoints
        .route("/deposits/simulate", post(simulate_deposit))
        .route("/deposits/process", post(process_deposits))
        .route("/withdrawals", post(request_withdrawal))
        .route("/withdrawals/pending", get(list_pending_withdrawals))
        // Status endpoint
        .route("/status", get(get_status))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Run the HTTP server
pub async fn serve(
    db_path: String,
    sequencer_url: Option<String>,
    port: u16,
) -> anyhow::Result<()> {
    // Initialize schema on startup
    let app = PredictionMarket::new(&db_path, sequencer_url.as_deref())?;
    drop(app); // Close the connection - we'll create new ones per request

    let router = create_router(db_path, sequencer_url);
    let addr = format!("0.0.0.0:{}", port);

    info!("Starting HTTP server on {}", addr);
    info!("Endpoints:");
    info!("  POST /accounts              - Create account");
    info!("  GET  /accounts              - List accounts");
    info!("  GET  /accounts/:id          - Get account");
    info!("  GET  /accounts/:id/positions - Get positions");
    info!("  POST /markets               - Create market");
    info!("  GET  /markets               - List markets");
    info!("  GET  /markets/:id           - Get market");
    info!("  POST /markets/:id/buy       - Buy shares");
    info!("  POST /markets/:id/sell      - Sell shares");
    info!("  POST /markets/:id/resolve   - Resolve market");
    info!("  POST /deposits/simulate     - Simulate deposit");
    info!("  POST /deposits/process      - Process deposits");
    info!("  POST /withdrawals           - Request withdrawal");
    info!("  GET  /withdrawals/pending   - List pending");
    info!("  GET  /status                - System status");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

// =============================================================================
// Request/Response types
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateAccountRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateMarketRequest {
    pub question: String,
    pub resolution_time: i64,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TradeRequest {
    pub account_id: i64,
    pub outcome: String,
    pub shares: i64,
}

#[derive(Debug, Deserialize)]
pub struct ResolveRequest {
    pub outcome: String,
}

#[derive(Debug, Deserialize)]
pub struct SimulateDepositRequest {
    pub tx_hash: String,
    pub from_address: String,
    pub to_address: String,
    pub amount: i64,
    #[serde(default = "default_block")]
    pub block_number: i64,
}

const fn default_block() -> i64 {
    1
}

#[derive(Debug, Deserialize)]
pub struct WithdrawalRequest {
    pub account_id: i64,
    pub amount: i64,
    pub destination_address: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IdResponse {
    pub id: i64,
}

#[derive(Debug, Serialize)]
pub struct CountResponse {
    pub count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub replicated: bool,
    pub healthy: bool,
    pub accounts: usize,
    pub markets: usize,
    pub pending_withdrawals: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<ReplicationStats>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReplicationStats {
    pub pending_changesets: usize,
    pub published_changesets: u64,
    pub failed_publishes: u64,
}

// =============================================================================
// Serializable wrappers for domain types
// =============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountJson {
    pub id: i64,
    pub name: String,
    pub balance: i64,
    pub balance_formatted: String,
    pub created_at: i64,
}

impl From<Account> for AccountJson {
    fn from(a: Account) -> Self {
        Self {
            id: a.id,
            name: a.name,
            balance: a.balance,
            balance_formatted: format!("${:.2}", a.balance as f64 / 100.0),
            created_at: a.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MarketJson {
    pub id: i64,
    pub question: String,
    pub description: Option<String>,
    pub resolution_time: i64,
    pub outcome: String,
    pub resolved_at: Option<i64>,
    pub created_at: i64,
}

impl From<Market> for MarketJson {
    fn from(m: Market) -> Self {
        Self {
            id: m.id,
            question: m.question,
            description: m.description,
            resolution_time: m.resolution_time,
            outcome: m.outcome,
            resolved_at: m.resolved_at,
            created_at: m.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PositionJson {
    pub id: i64,
    pub account_id: i64,
    pub market_id: i64,
    pub outcome: String,
    pub shares: i64,
    pub cost_basis: i64,
}

impl From<Position> for PositionJson {
    fn from(p: Position) -> Self {
        Self {
            id: p.id,
            account_id: p.account_id,
            market_id: p.market_id,
            outcome: p.outcome,
            shares: p.shares,
            cost_basis: p.cost_basis,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TradeJson {
    pub id: i64,
    pub account_id: i64,
    pub market_id: i64,
    pub outcome: String,
    pub side: String,
    pub shares: i64,
    pub price: i64,
    pub total: i64,
}

impl From<Trade> for TradeJson {
    fn from(t: Trade) -> Self {
        Self {
            id: t.id,
            account_id: t.account_id,
            market_id: t.market_id,
            outcome: t.outcome,
            side: t.side,
            shares: t.shares,
            price: t.price,
            total: t.total,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WithdrawalJson {
    pub id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub amount: i64,
    pub amount_formatted: String,
    pub destination_address: String,
    pub status: String,
}

impl From<Withdrawal> for WithdrawalJson {
    fn from(w: Withdrawal) -> Self {
        Self {
            id: w.id,
            account_id: w.account_id,
            account_name: w.account_name,
            amount: w.amount,
            amount_formatted: format!("${:.2}", w.amount as f64 / 100.0),
            destination_address: w.destination_address,
            status: w.status,
        }
    }
}

// =============================================================================
// Error handling
// =============================================================================

/// Application error type that converts to HTTP responses
#[derive(Debug)]
pub struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        #[derive(Serialize)]
        struct ErrorResponse {
            error: String,
        }

        let status = if self.0.to_string().contains("not found") {
            StatusCode::NOT_FOUND
        } else if self.0.to_string().contains("insufficient")
            || self.0.to_string().contains("already")
        {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };

        (
            status,
            Json(ErrorResponse {
                error: self.0.to_string(),
            }),
        )
            .into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

// =============================================================================
// Handlers
// =============================================================================

async fn create_account(
    State(config): State<AppState>,
    Json(req): Json<CreateAccountRequest>,
) -> Result<Json<IdResponse>, AppError> {
    let app = config.connect()?;
    let id = app.create_account(&req.name)?;
    app.publish()?;
    Ok(Json(IdResponse { id }))
}

async fn list_accounts(State(config): State<AppState>) -> Result<Json<Vec<AccountJson>>, AppError> {
    let app = config.connect()?;
    let accounts = app.list_accounts()?;
    Ok(Json(accounts.into_iter().map(Into::into).collect()))
}

async fn get_account(
    State(config): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<AccountJson>, AppError> {
    let app = config.connect()?;
    let account = app.get_account(id)?;
    Ok(Json(account.into()))
}

async fn get_positions(
    State(config): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<PositionJson>>, AppError> {
    let app = config.connect()?;
    let positions = app.get_positions(id)?;
    Ok(Json(positions.into_iter().map(Into::into).collect()))
}

async fn create_market(
    State(config): State<AppState>,
    Json(req): Json<CreateMarketRequest>,
) -> Result<Json<IdResponse>, AppError> {
    let app = config.connect()?;
    let id = app.create_market(
        &req.question,
        req.description.as_deref(),
        req.resolution_time,
    )?;
    app.publish()?;
    Ok(Json(IdResponse { id }))
}

async fn list_markets(State(config): State<AppState>) -> Result<Json<Vec<MarketJson>>, AppError> {
    let app = config.connect()?;
    let markets = app.list_markets()?;
    Ok(Json(markets.into_iter().map(Into::into).collect()))
}

async fn get_market(
    State(config): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<MarketJson>, AppError> {
    let app = config.connect()?;
    let market = app.get_market(id)?;
    Ok(Json(market.into()))
}

async fn buy_shares(
    State(config): State<AppState>,
    Path(market_id): Path<i64>,
    Json(req): Json<TradeRequest>,
) -> Result<Json<TradeJson>, AppError> {
    let app = config.connect()?;
    let trade = app.buy_shares(req.account_id, market_id, &req.outcome, req.shares)?;
    app.publish()?;
    Ok(Json(trade.into()))
}

async fn sell_shares(
    State(config): State<AppState>,
    Path(market_id): Path<i64>,
    Json(req): Json<TradeRequest>,
) -> Result<Json<TradeJson>, AppError> {
    let app = config.connect()?;
    let trade = app.sell_shares(req.account_id, market_id, &req.outcome, req.shares)?;
    app.publish()?;
    Ok(Json(trade.into()))
}

async fn resolve_market(
    State(config): State<AppState>,
    Path(market_id): Path<i64>,
    Json(req): Json<ResolveRequest>,
) -> Result<StatusCode, AppError> {
    let app = config.connect()?;
    app.resolve_market(market_id, &req.outcome)?;
    app.publish()?;
    Ok(StatusCode::OK)
}

async fn simulate_deposit(
    State(config): State<AppState>,
    Json(req): Json<SimulateDepositRequest>,
) -> Result<Json<IdResponse>, AppError> {
    let app = config.connect()?;
    let id = app.simulate_deposit(
        &req.tx_hash,
        &req.from_address,
        &req.to_address,
        req.amount,
        req.block_number,
    )?;
    app.publish()?;
    Ok(Json(IdResponse { id }))
}

async fn process_deposits(State(config): State<AppState>) -> Result<Json<CountResponse>, AppError> {
    let app = config.connect()?;
    let count = app.process_deposits()?;
    app.publish()?;
    Ok(Json(CountResponse { count }))
}

async fn request_withdrawal(
    State(config): State<AppState>,
    Json(req): Json<WithdrawalRequest>,
) -> Result<Json<IdResponse>, AppError> {
    let app = config.connect()?;
    let id = app.request_withdrawal(req.account_id, req.amount, &req.destination_address)?;
    app.publish()?;
    Ok(Json(IdResponse { id }))
}

async fn list_pending_withdrawals(
    State(config): State<AppState>,
) -> Result<Json<Vec<WithdrawalJson>>, AppError> {
    let app = config.connect()?;
    let withdrawals = app.list_pending_withdrawals()?;
    Ok(Json(withdrawals.into_iter().map(Into::into).collect()))
}

async fn get_status(State(config): State<AppState>) -> Result<Json<StatusResponse>, AppError> {
    let app = config.connect()?;
    let accounts = app.list_accounts()?.len();
    let markets = app.list_markets()?.len();
    let pending_withdrawals = app.list_pending_withdrawals()?.len();

    let stats = app.stats().map(|s| ReplicationStats {
        pending_changesets: s.pending_changesets,
        published_changesets: s.published_changesets,
        failed_publishes: s.failed_publishes,
    });

    Ok(Json(StatusResponse {
        replicated: app.is_replicated(),
        healthy: app.is_healthy(),
        accounts,
        markets,
        pending_withdrawals,
        stats,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use std::sync::atomic::{AtomicU64, Ordering};
    use tower::ServiceExt;

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_router() -> Router {
        // Use a unique temp file for each test to avoid conflicts
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let db_path = format!(
            "/tmp/prediction_market_test_{}_{}.db",
            std::process::id(),
            counter
        );
        // Remove any existing file
        let _ = std::fs::remove_file(&db_path);
        // Initialize schema
        let _ = PredictionMarket::new(&db_path, None).unwrap();
        create_router(db_path, None)
    }

    #[tokio::test]
    async fn test_create_and_get_account() {
        let router = test_router();

        // Create account
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/accounts")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name": "alice"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let id_resp: IdResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(id_resp.id, 1);

        // Get account
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/accounts/1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let account: AccountJson = serde_json::from_slice(&body).unwrap();
        assert_eq!(account.name, "alice");
        assert_eq!(account.balance, 1_000_000);
    }

    #[tokio::test]
    async fn test_create_market_and_trade() {
        let router = test_router();

        // Create account
        router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/accounts")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name": "trader"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Create market
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/markets")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"question": "Will BTC hit 100k?", "resolution_time": 1800000000}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Buy shares
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/markets/1/buy")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"account_id": 1, "outcome": "yes", "shares": 100}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let trade: TradeJson = serde_json::from_slice(&body).unwrap();
        assert_eq!(trade.shares, 100);
        assert_eq!(trade.total, 5000);
    }

    #[tokio::test]
    async fn test_status_endpoint() {
        let router = test_router();

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let status: StatusResponse = serde_json::from_slice(&body).unwrap();
        assert!(!status.replicated);
    }
}
