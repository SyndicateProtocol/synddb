# SyndDB Extension Development Guide

## Philosophy

SyndDB follows a **Core + Extensions** architecture. The **SyndDB Core** provides the high-performance database engine, state replication, and blockchain integration. **SyndDB Extensions** are developer-created modules that add specific business logic, schemas, and functionality on top of the Core.

This separation enables:
- **Core stability**: The SyndDB Core engine remains unchanged across different use cases
- **Extension flexibility**: Each extension implements only its specific requirements
- **Rapid development**: No need to understand Core internals to build extensions
- **Type safety**: Strongly-typed interfaces ensure correctness at compile time
- **Performance optimization**: Core optimizations automatically benefit all extensions

## Architecture Overview

```
┌──────────────────────────────────────────────────────────┐
│                     Extensions Layer                      │
│  (Developer Extensions - Schemas, Writes, Rules, Bridges) │
├──────────────────────────────────────────────────────────┤
│                   Extension Interface                     │
│      (Traits, Registries, Hooks, Validators, APIs)       │
├──────────────────────────────────────────────────────────┤
│                      SyndDB Core                         │
│  (SQLite Engine, Sequencer, Replication, Blockchain)     │
└──────────────────────────────────────────────────────────┘
```

## Extension Points

### 1. Schema Extensions

Extensions define their database schema as SQL DDL statements. The SyndDB Core handles execution and replication - extensions just provide the schema.

```rust
// synddb-extensions/src/schema.rs
pub trait SchemaExtension: Send + Sync {
    /// Unique identifier for this schema
    fn schema_id(&self) -> &str;

    /// Version for migrations
    fn version(&self) -> u32;

    /// SQL DDL statements to create the schema
    fn create_statements(&self) -> Vec<String>;

    /// SQL statements to migrate from a previous version
    fn migrate_statements(&self, from_version: u32) -> Result<Vec<String>>;

    /// Indexes to create for optimal performance
    fn index_statements(&self) -> Vec<String>;

    /// Initial data to seed
    fn seed_statements(&self) -> Vec<String>;
}
```

### 2. LocalWrite Type Extensions

Define custom write operations that map to SQL statements. The SyndDB Core handles validation, serialization, and execution.

```rust
// synddb-extensions/src/writes.rs
pub trait LocalWriteExtension: Send + Sync {
    /// The write type this extension handles
    fn write_type(&self) -> &str;

    /// JSON schema for validation
    fn schema(&self) -> &serde_json::Value;

    /// Validate a write request
    fn validate(&self, request: &serde_json::Value) -> Result<()>;

    /// Convert the write request to SQL statements
    fn to_sql(&self, request: &serde_json::Value) -> Result<Vec<SqlStatement>>;

    /// Optional pre-execution hook
    fn pre_execute(&self, request: &serde_json::Value) -> Result<()> {
        Ok(())
    }

    /// Optional post-execution hook
    fn post_execute(&self, request: &serde_json::Value, result: &ExecuteResult) -> Result<()> {
        Ok(())
    }
}

pub struct SqlStatement {
    pub sql: String,
    pub params: Vec<rusqlite::types::Value>,
}
```

### 3. Trigger Extensions

Extensions register SQLite triggers for automatic business logic execution at the database level.

```rust
// synddb-extensions/src/triggers.rs
pub trait TriggerExtension: Send + Sync {
    /// Unique identifier for this trigger
    fn trigger_id(&self) -> &str;

    /// The table this trigger applies to
    fn table_name(&self) -> &str;

    /// When the trigger fires (BEFORE/AFTER INSERT/UPDATE/DELETE)
    fn trigger_event(&self) -> TriggerEvent;

    /// SQL body of the trigger
    fn trigger_sql(&self) -> String;

    /// Dependencies on other triggers (for ordering)
    fn dependencies(&self) -> Vec<String> {
        vec![]
    }
}

pub enum TriggerEvent {
    BeforeInsert,
    AfterInsert,
    BeforeUpdate,
    AfterUpdate,
    BeforeDelete,
    AfterDelete,
}
```

### 4. Bridge Extensions

Define how your extension interacts with external blockchains for deposits and withdrawals.

```rust
// synddb-extensions/src/bridge.rs
pub trait BridgeExtension: Send + Sync {
    /// Unique identifier for this bridge
    fn bridge_id(&self) -> &str;

    /// Process incoming deposits from the settlement chain
    async fn process_deposit(&self, deposit: BridgeDeposit) -> Result<Vec<SqlStatement>>;

    /// Validate and prepare withdrawal requests
    async fn prepare_withdrawal(&self, withdrawal: WithdrawalRequest) -> Result<BridgeMessage>;

    /// Generate proof for validator attestation
    async fn generate_proof(&self, message: &BridgeMessage) -> Result<Vec<u8>>;

    /// Verify proof from validator
    async fn verify_proof(&self, message: &BridgeMessage, proof: &[u8]) -> Result<bool>;
}

pub struct BridgeDeposit {
    pub from_address: String,
    pub to_account: String,
    pub asset: String,
    pub amount: String,
    pub tx_hash: String,
    pub metadata: serde_json::Value,
}

pub struct WithdrawalRequest {
    pub request_id: String,
    pub from_account: String,
    pub to_address: String,
    pub asset: String,
    pub amount: String,
    pub metadata: serde_json::Value,
}
```

### 5. Query Extensions

Define custom query patterns and caching strategies for your extension.

```rust
// synddb-extensions/src/queries.rs
pub trait QueryExtension: Send + Sync {
    /// Unique identifier for this query type
    fn query_id(&self) -> &str;

    /// Transform a high-level query request into SQL
    fn to_sql(&self, request: &QueryRequest) -> Result<SqlQuery>;

    /// Cache key generation for this query
    fn cache_key(&self, request: &QueryRequest) -> String;

    /// Cache TTL in milliseconds
    fn cache_ttl_ms(&self) -> u64;

    /// Post-process query results
    fn process_results(&self, results: Vec<rusqlite::Row>) -> Result<serde_json::Value>;
}
```

## Extension Registration

Extensions are registered at startup through a builder pattern:

```rust
// main.rs
use synddb::ExtensionRegistry;
use my_app::{OrderBookSchema, PlaceOrderWrite, OrderMatchingTrigger, TokenBridge};

#[tokio::main]
async fn main() -> Result<()> {
    let mut registry = ExtensionRegistry::new();

    // Register schemas
    registry.register_schema(Box::new(OrderBookSchema::new()))?;

    // Register write types
    registry.register_write(Box::new(PlaceOrderWrite::new()))?;
    registry.register_write(Box::new(CancelOrderWrite::new()))?;

    // Register triggers
    registry.register_trigger(Box::new(OrderMatchingTrigger::new()))?;

    // Register bridge
    registry.register_bridge(Box::new(TokenBridge::new()))?;

    // Start the node with extensions
    let config = Config::from_env()?;
    let node = match config.role {
        Role::Sequencer => SequencerNode::with_extensions(config, registry),
        Role::Replica => ReplicaNode::with_extensions(config, registry),
    };

    node.start().await?;
    Ok(())
}
```

## Example Extensions

### 1. Perpetual DEX Extension

A complete perpetual DEX extension showcasing all extension points:

```rust
// perp-dex-extension/src/lib.rs

pub struct PerpDexExtension;

impl SchemaExtension for PerpDexExtension {
    fn schema_id(&self) -> &str { "perp-dex-v1" }

    fn create_statements(&self) -> Vec<String> {
        vec![
            // Order book tables
            r#"CREATE TABLE orders (
                order_id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                market TEXT NOT NULL,
                side TEXT CHECK(side IN ('LONG', 'SHORT')),
                order_type TEXT CHECK(order_type IN ('MARKET', 'LIMIT', 'STOP')),
                price REAL,
                quantity REAL NOT NULL,
                leverage INTEGER DEFAULT 1,
                remaining_quantity REAL NOT NULL,
                margin_required REAL NOT NULL,
                status TEXT CHECK(status IN ('PENDING', 'OPEN', 'PARTIAL', 'FILLED', 'CANCELED', 'LIQUIDATED')),
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                expiry INTEGER,
                reduce_only BOOLEAN DEFAULT FALSE,
                post_only BOOLEAN DEFAULT FALSE
            )"#.to_string(),

            // Position tracking
            r#"CREATE TABLE positions (
                position_id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                market TEXT NOT NULL,
                side TEXT CHECK(side IN ('LONG', 'SHORT')),
                entry_price REAL NOT NULL,
                mark_price REAL NOT NULL,
                quantity REAL NOT NULL,
                leverage INTEGER NOT NULL,
                margin REAL NOT NULL,
                unrealized_pnl REAL NOT NULL,
                liquidation_price REAL NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                UNIQUE(account_id, market)
            )"#.to_string(),

            // Trade history
            r#"CREATE TABLE trades (
                trade_id TEXT PRIMARY KEY,
                market TEXT NOT NULL,
                buy_order_id TEXT NOT NULL,
                sell_order_id TEXT NOT NULL,
                price REAL NOT NULL,
                quantity REAL NOT NULL,
                timestamp INTEGER NOT NULL,
                taker_side TEXT CHECK(taker_side IN ('BUY', 'SELL')),
                FOREIGN KEY (buy_order_id) REFERENCES orders(order_id),
                FOREIGN KEY (sell_order_id) REFERENCES orders(order_id)
            )"#.to_string(),

            // Market data
            r#"CREATE TABLE market_data (
                market TEXT PRIMARY KEY,
                last_price REAL NOT NULL,
                mark_price REAL NOT NULL,
                index_price REAL NOT NULL,
                funding_rate REAL NOT NULL,
                next_funding_time INTEGER NOT NULL,
                open_interest REAL NOT NULL,
                volume_24h REAL NOT NULL,
                updated_at INTEGER NOT NULL
            )"#.to_string(),

            // Liquidations
            r#"CREATE TABLE liquidations (
                liquidation_id TEXT PRIMARY KEY,
                position_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                market TEXT NOT NULL,
                side TEXT NOT NULL,
                quantity REAL NOT NULL,
                price REAL NOT NULL,
                margin_lost REAL NOT NULL,
                timestamp INTEGER NOT NULL,
                FOREIGN KEY (position_id) REFERENCES positions(position_id)
            )"#.to_string(),

            // Funding payments
            r#"CREATE TABLE funding_payments (
                payment_id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                market TEXT NOT NULL,
                position_size REAL NOT NULL,
                funding_rate REAL NOT NULL,
                payment_amount REAL NOT NULL,
                timestamp INTEGER NOT NULL
            )"#.to_string(),
        ]
    }

    fn index_statements(&self) -> Vec<String> {
        vec![
            "CREATE INDEX idx_orders_market_status ON orders(market, status, price)".to_string(),
            "CREATE INDEX idx_orders_account ON orders(account_id, status)".to_string(),
            "CREATE INDEX idx_positions_liquidation ON positions(liquidation_price, market)".to_string(),
            "CREATE INDEX idx_trades_market_time ON trades(market, timestamp DESC)".to_string(),
        ]
    }
}

// Order matching trigger
impl TriggerExtension for OrderMatchingTrigger {
    fn trigger_id(&self) -> &str { "perp-order-matching" }
    fn table_name(&self) -> &str { "orders" }
    fn trigger_event(&self) -> TriggerEvent { TriggerEvent::AfterInsert }

    fn trigger_sql(&self) -> String {
        r#"
        BEGIN
            -- Find matching orders on opposite side
            WITH matches AS (
                SELECT
                    o.order_id,
                    o.price,
                    o.remaining_quantity,
                    MIN(NEW.remaining_quantity, o.remaining_quantity) as match_quantity
                FROM orders o
                WHERE o.market = NEW.market
                    AND o.status IN ('OPEN', 'PARTIAL')
                    AND o.side != NEW.side
                    AND (
                        (NEW.order_type = 'MARKET') OR
                        (NEW.side = 'LONG' AND o.price <= NEW.price) OR
                        (NEW.side = 'SHORT' AND o.price >= NEW.price)
                    )
                ORDER BY
                    CASE WHEN NEW.side = 'LONG' THEN o.price END ASC,
                    CASE WHEN NEW.side = 'SHORT' THEN o.price END DESC,
                    o.created_at ASC
                LIMIT 1
            )
            INSERT INTO trades (trade_id, market, buy_order_id, sell_order_id, price, quantity, timestamp, taker_side)
            SELECT
                hex(randomblob(16)),
                NEW.market,
                CASE WHEN NEW.side = 'LONG' THEN NEW.order_id ELSE order_id END,
                CASE WHEN NEW.side = 'SHORT' THEN NEW.order_id ELSE order_id END,
                CASE WHEN NEW.order_type = 'MARKET' THEN price ELSE NEW.price END,
                match_quantity,
                strftime('%s', 'now'),
                NEW.side
            FROM matches
            WHERE match_quantity > 0;

            -- Update matched orders
            UPDATE orders
            SET
                remaining_quantity = remaining_quantity - (
                    SELECT match_quantity FROM matches WHERE matches.order_id = orders.order_id
                ),
                status = CASE
                    WHEN remaining_quantity = 0 THEN 'FILLED'
                    ELSE 'PARTIAL'
                END,
                updated_at = strftime('%s', 'now')
            WHERE order_id IN (
                SELECT order_id FROM matches
                UNION SELECT NEW.order_id
            );

            -- Update or create positions
            INSERT INTO positions (
                position_id, account_id, market, side, entry_price, mark_price,
                quantity, leverage, margin, unrealized_pnl, liquidation_price,
                created_at, updated_at
            )
            SELECT
                hex(randomblob(16)),
                NEW.account_id,
                NEW.market,
                NEW.side,
                (SELECT price FROM matches),
                (SELECT mark_price FROM market_data WHERE market = NEW.market),
                (SELECT match_quantity FROM matches),
                NEW.leverage,
                NEW.margin_required * (SELECT match_quantity FROM matches) / NEW.quantity,
                0,
                CASE
                    WHEN NEW.side = 'LONG' THEN
                        (SELECT price FROM matches) * (1 - 1.0 / NEW.leverage)
                    ELSE
                        (SELECT price FROM matches) * (1 + 1.0 / NEW.leverage)
                END,
                strftime('%s', 'now'),
                strftime('%s', 'now')
            WHERE EXISTS (SELECT 1 FROM matches WHERE match_quantity > 0)
            ON CONFLICT (account_id, market) DO UPDATE SET
                quantity = positions.quantity + excluded.quantity,
                entry_price = (positions.entry_price * positions.quantity + excluded.entry_price * excluded.quantity)
                             / (positions.quantity + excluded.quantity),
                margin = positions.margin + excluded.margin,
                liquidation_price = excluded.liquidation_price,
                updated_at = excluded.updated_at;
        END;
        "#.to_string()
    }
}

// Liquidation monitoring trigger
impl TriggerExtension for LiquidationTrigger {
    fn trigger_id(&self) -> &str { "perp-liquidation-monitor" }
    fn table_name(&self) -> &str { "market_data" }
    fn trigger_event(&self) -> TriggerEvent { TriggerEvent::AfterUpdate }

    fn trigger_sql(&self) -> String {
        r#"
        BEGIN
            -- Check for positions that should be liquidated
            INSERT INTO liquidations (
                liquidation_id, position_id, account_id, market, side,
                quantity, price, margin_lost, timestamp
            )
            SELECT
                hex(randomblob(16)),
                p.position_id,
                p.account_id,
                p.market,
                p.side,
                p.quantity,
                NEW.mark_price,
                p.margin,
                strftime('%s', 'now')
            FROM positions p
            WHERE p.market = NEW.market
                AND (
                    (p.side = 'LONG' AND NEW.mark_price <= p.liquidation_price) OR
                    (p.side = 'SHORT' AND NEW.mark_price >= p.liquidation_price)
                );

            -- Close liquidated positions
            DELETE FROM positions
            WHERE position_id IN (
                SELECT position_id FROM liquidations
                WHERE timestamp = strftime('%s', 'now')
            );

            -- Cancel open orders for liquidated accounts
            UPDATE orders
            SET status = 'CANCELED', updated_at = strftime('%s', 'now')
            WHERE account_id IN (
                SELECT DISTINCT account_id FROM liquidations
                WHERE timestamp = strftime('%s', 'now')
            )
            AND status IN ('OPEN', 'PARTIAL');
        END;
        "#.to_string()
    }
}
```

### 2. ERC-20 Token Extension

A complete ERC-20 implementation with minting, burning, and bridging:

```rust
// erc20-extension/src/lib.rs

pub struct ERC20Extension {
    supported_tokens: Vec<TokenConfig>,
}

impl SchemaExtension for ERC20Extension {
    fn schema_id(&self) -> &str { "erc20-v1" }

    fn create_statements(&self) -> Vec<String> {
        vec![
            r#"CREATE TABLE token_metadata (
                token_address TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                symbol TEXT NOT NULL,
                decimals INTEGER NOT NULL,
                total_supply TEXT NOT NULL,
                owner TEXT,
                paused BOOLEAN DEFAULT FALSE,
                created_at INTEGER NOT NULL
            )"#.to_string(),

            r#"CREATE TABLE balances (
                account_id TEXT NOT NULL,
                token_address TEXT NOT NULL,
                balance TEXT NOT NULL,
                locked_balance TEXT DEFAULT '0',
                nonce INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (account_id, token_address),
                FOREIGN KEY (token_address) REFERENCES token_metadata(token_address)
            )"#.to_string(),

            r#"CREATE TABLE allowances (
                owner TEXT NOT NULL,
                spender TEXT NOT NULL,
                token_address TEXT NOT NULL,
                amount TEXT NOT NULL,
                expiry INTEGER,
                PRIMARY KEY (owner, spender, token_address),
                FOREIGN KEY (token_address) REFERENCES token_metadata(token_address)
            )"#.to_string(),

            r#"CREATE TABLE transfer_events (
                event_id TEXT PRIMARY KEY,
                token_address TEXT NOT NULL,
                from_address TEXT NOT NULL,
                to_address TEXT NOT NULL,
                amount TEXT NOT NULL,
                memo TEXT,
                timestamp INTEGER NOT NULL,
                block_number INTEGER,
                transaction_index INTEGER
            )"#.to_string(),

            r#"CREATE TABLE mint_burn_events (
                event_id TEXT PRIMARY KEY,
                token_address TEXT NOT NULL,
                event_type TEXT CHECK(event_type IN ('MINT', 'BURN')),
                account TEXT NOT NULL,
                amount TEXT NOT NULL,
                authority TEXT NOT NULL,
                timestamp INTEGER NOT NULL
            )"#.to_string(),

            r#"CREATE TABLE withdrawal_requests (
                request_id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                token_address TEXT NOT NULL,
                amount TEXT NOT NULL,
                destination_address TEXT NOT NULL,
                status TEXT CHECK(status IN ('PENDING', 'PROCESSING', 'COMPLETED', 'FAILED')),
                created_at INTEGER NOT NULL,
                processed_at INTEGER,
                settlement_tx_hash TEXT,
                error_message TEXT
            )"#.to_string(),

            r#"CREATE TABLE deposit_records (
                deposit_id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                token_address TEXT NOT NULL,
                amount TEXT NOT NULL,
                source_tx_hash TEXT NOT NULL,
                source_address TEXT NOT NULL,
                confirmations INTEGER DEFAULT 0,
                timestamp INTEGER NOT NULL,
                status TEXT CHECK(status IN ('PENDING', 'CONFIRMED', 'FAILED'))
            )"#.to_string(),
        ]
    }
}

// Transfer write operation
impl LocalWriteExtension for TransferWrite {
    fn write_type(&self) -> &str { "erc20.transfer" }

    fn schema(&self) -> &serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["from", "to", "token", "amount"],
            "properties": {
                "from": {"type": "string"},
                "to": {"type": "string"},
                "token": {"type": "string"},
                "amount": {"type": "string"},
                "memo": {"type": "string"}
            }
        })
    }

    fn validate(&self, request: &serde_json::Value) -> Result<()> {
        // Validate addresses
        let from = request["from"].as_str().ok_or("Invalid from address")?;
        let to = request["to"].as_str().ok_or("Invalid to address")?;
        let amount = request["amount"].as_str().ok_or("Invalid amount")?;

        // Check amount is positive
        let amount_val: u128 = amount.parse().map_err(|_| "Invalid amount format")?;
        if amount_val == 0 {
            return Err(anyhow!("Amount must be greater than zero"));
        }

        Ok(())
    }

    fn to_sql(&self, request: &serde_json::Value) -> Result<Vec<SqlStatement>> {
        let from = request["from"].as_str().unwrap();
        let to = request["to"].as_str().unwrap();
        let token = request["token"].as_str().unwrap();
        let amount = request["amount"].as_str().unwrap();
        let memo = request.get("memo").and_then(|v| v.as_str()).unwrap_or("");

        Ok(vec![
            // Debit sender
            SqlStatement {
                sql: "UPDATE balances SET balance = CAST(CAST(balance AS INTEGER) - CAST(?1 AS INTEGER) AS TEXT)
                      WHERE account_id = ?2 AND token_address = ?3 AND CAST(balance AS INTEGER) >= CAST(?1 AS INTEGER)".to_string(),
                params: vec![amount.into(), from.into(), token.into()],
            },
            // Credit receiver
            SqlStatement {
                sql: "INSERT INTO balances (account_id, token_address, balance, locked_balance, nonce)
                      VALUES (?1, ?2, ?3, '0', 0)
                      ON CONFLICT (account_id, token_address)
                      DO UPDATE SET balance = CAST(CAST(balance AS INTEGER) + CAST(?3 AS INTEGER) AS TEXT)".to_string(),
                params: vec![to.into(), token.into(), amount.into()],
            },
            // Record event
            SqlStatement {
                sql: "INSERT INTO transfer_events (event_id, token_address, from_address, to_address, amount, memo, timestamp)
                      VALUES (hex(randomblob(16)), ?1, ?2, ?3, ?4, ?5, strftime('%s', 'now'))".to_string(),
                params: vec![token.into(), from.into(), to.into(), amount.into(), memo.into()],
            },
        ])
    }
}

// Balance protection trigger
impl TriggerExtension for BalanceProtectionTrigger {
    fn trigger_id(&self) -> &str { "erc20-balance-protection" }
    fn table_name(&self) -> &str { "balances" }
    fn trigger_event(&self) -> TriggerEvent { TriggerEvent::BeforeUpdate }

    fn trigger_sql(&self) -> String {
        r#"
        BEGIN
            SELECT CASE
                WHEN NEW.balance < '0' THEN
                    RAISE(ABORT, 'Insufficient balance for transfer')
                WHEN NEW.balance < NEW.locked_balance THEN
                    RAISE(ABORT, 'Balance cannot be less than locked amount')
            END;
        END;
        "#.to_string()
    }
}

// Token bridge for deposits and withdrawals
impl BridgeExtension for ERC20Bridge {
    fn bridge_id(&self) -> &str { "erc20-bridge-v1" }

    async fn process_deposit(&self, deposit: BridgeDeposit) -> Result<Vec<SqlStatement>> {
        // Verify the deposit is from a supported token contract
        if !self.is_supported_token(&deposit.asset) {
            return Err(anyhow!("Unsupported token"));
        }

        Ok(vec![
            // Credit the account
            SqlStatement {
                sql: "INSERT INTO balances (account_id, token_address, balance, locked_balance, nonce)
                      VALUES (?1, ?2, ?3, '0', 0)
                      ON CONFLICT (account_id, token_address)
                      DO UPDATE SET balance = CAST(CAST(balance AS INTEGER) + CAST(?3 AS INTEGER) AS TEXT)".to_string(),
                params: vec![
                    deposit.to_account.clone().into(),
                    deposit.asset.clone().into(),
                    deposit.amount.clone().into(),
                ],
            },
            // Record the deposit
            SqlStatement {
                sql: "INSERT INTO deposit_records (deposit_id, account_id, token_address, amount, source_tx_hash, source_address, timestamp, status)
                      VALUES (hex(randomblob(16)), ?1, ?2, ?3, ?4, ?5, strftime('%s', 'now'), 'CONFIRMED')".to_string(),
                params: vec![
                    deposit.to_account.into(),
                    deposit.asset.clone().into(),
                    deposit.amount.clone().into(),
                    deposit.tx_hash.into(),
                    deposit.from_address.into(),
                ],
            },
            // Record as mint event
            SqlStatement {
                sql: "INSERT INTO transfer_events (event_id, token_address, from_address, to_address, amount, memo, timestamp)
                      VALUES (hex(randomblob(16)), ?1, '0x0', ?2, ?3, 'Bridge Deposit', strftime('%s', 'now'))".to_string(),
                params: vec![
                    deposit.asset.into(),
                    deposit.to_account.into(),
                    deposit.amount.into(),
                ],
            },
        ])
    }

    async fn prepare_withdrawal(&self, withdrawal: WithdrawalRequest) -> Result<BridgeMessage> {
        // Create the withdrawal message for validators to sign
        Ok(BridgeMessage {
            message_type: "erc20_withdrawal".to_string(),
            nonce: generate_nonce(),
            payload: serde_json::json!({
                "request_id": withdrawal.request_id,
                "token": withdrawal.asset,
                "amount": withdrawal.amount,
                "recipient": withdrawal.to_address,
                "timestamp": SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            }),
        })
    }
}
```

### 3. Gaming Leaderboard Extension

A lightweight gaming extension showcasing simple schemas and triggers:

```rust
// gaming-extension/src/lib.rs

pub struct GamingExtension;

impl SchemaExtension for GamingExtension {
    fn schema_id(&self) -> &str { "gaming-v1" }

    fn create_statements(&self) -> Vec<String> {
        vec![
            r#"CREATE TABLE players (
                player_id TEXT PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                display_name TEXT NOT NULL,
                level INTEGER DEFAULT 1,
                experience INTEGER DEFAULT 0,
                total_score INTEGER DEFAULT 0,
                games_played INTEGER DEFAULT 0,
                games_won INTEGER DEFAULT 0,
                win_rate REAL DEFAULT 0,
                created_at INTEGER NOT NULL,
                last_active INTEGER NOT NULL
            )"#.to_string(),

            r#"CREATE TABLE games (
                game_id TEXT PRIMARY KEY,
                game_type TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                ended_at INTEGER,
                status TEXT CHECK(status IN ('WAITING', 'ACTIVE', 'COMPLETED', 'ABANDONED')),
                max_players INTEGER NOT NULL,
                current_players INTEGER DEFAULT 0
            )"#.to_string(),

            r#"CREATE TABLE game_results (
                result_id TEXT PRIMARY KEY,
                game_id TEXT NOT NULL,
                player_id TEXT NOT NULL,
                score INTEGER NOT NULL,
                placement INTEGER NOT NULL,
                rewards_earned TEXT,
                timestamp INTEGER NOT NULL,
                FOREIGN KEY (game_id) REFERENCES games(game_id),
                FOREIGN KEY (player_id) REFERENCES players(player_id)
            )"#.to_string(),

            r#"CREATE TABLE leaderboards (
                leaderboard_id TEXT PRIMARY KEY,
                period TEXT CHECK(period IN ('DAILY', 'WEEKLY', 'MONTHLY', 'ALL_TIME')),
                game_type TEXT,
                player_id TEXT NOT NULL,
                score INTEGER NOT NULL,
                rank INTEGER NOT NULL,
                timestamp INTEGER NOT NULL,
                UNIQUE(period, game_type, player_id)
            )"#.to_string(),
        ]
    }

    fn index_statements(&self) -> Vec<String> {
        vec![
            "CREATE INDEX idx_leaderboards_ranking ON leaderboards(period, game_type, score DESC)".to_string(),
            "CREATE INDEX idx_game_results_player ON game_results(player_id, timestamp DESC)".to_string(),
        ]
    }
}

// Auto-update leaderboards trigger
impl TriggerExtension for LeaderboardUpdateTrigger {
    fn trigger_id(&self) -> &str { "gaming-leaderboard-update" }
    fn table_name(&self) -> &str { "game_results" }
    fn trigger_event(&self) -> TriggerEvent { TriggerEvent::AfterInsert }

    fn trigger_sql(&self) -> String {
        r#"
        BEGIN
            -- Update player stats
            UPDATE players
            SET
                total_score = total_score + NEW.score,
                games_played = games_played + 1,
                games_won = games_won + CASE WHEN NEW.placement = 1 THEN 1 ELSE 0 END,
                win_rate = CAST(games_won AS REAL) / CAST(games_played AS REAL),
                experience = experience + (NEW.score / 10),
                level = 1 + (experience / 1000),
                last_active = strftime('%s', 'now')
            WHERE player_id = NEW.player_id;

            -- Update daily leaderboard
            INSERT INTO leaderboards (leaderboard_id, period, game_type, player_id, score, rank, timestamp)
            VALUES (
                hex(randomblob(16)),
                'DAILY',
                (SELECT game_type FROM games WHERE game_id = NEW.game_id),
                NEW.player_id,
                NEW.score,
                0,
                strftime('%s', 'now')
            )
            ON CONFLICT (period, game_type, player_id) DO UPDATE SET
                score = score + NEW.score,
                timestamp = strftime('%s', 'now');

            -- Recalculate ranks
            UPDATE leaderboards
            SET rank = (
                SELECT COUNT(*) + 1
                FROM leaderboards l2
                WHERE l2.period = leaderboards.period
                    AND l2.game_type = leaderboards.game_type
                    AND l2.score > leaderboards.score
            )
            WHERE period = 'DAILY'
                AND game_type = (SELECT game_type FROM games WHERE game_id = NEW.game_id);
        END;
        "#.to_string()
    }
}
```

## Extension Development Guide

### Step 1: Create Your Extension Crate

```toml
# Cargo.toml
[package]
name = "my-synddb-extension"
version = "0.1.0"

[dependencies]
synddb-extensions = "0.1"
rusqlite = "0.30"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
async-trait = "0.1"
anyhow = "1.0"
```

### Step 2: Implement Extension Traits

```rust
use synddb_extensions::*;

pub struct MyExtension {
    config: MyConfig,
}

impl MyExtension {
    pub fn new(config: MyConfig) -> Self {
        Self { config }
    }
}

// Implement the traits you need
impl SchemaExtension for MyExtension { /* ... */ }
impl LocalWriteExtension for MyExtension { /* ... */ }
impl TriggerExtension for MyExtension { /* ... */ }
```

### Step 3: Register with SyndDB

```rust
use synddb::{SequencerNode, ExtensionRegistry};
use my_extension::MyExtension;

let mut registry = ExtensionRegistry::new();
registry.register_extension(Box::new(MyExtension::new(config)))?;

let node = SequencerNode::with_extensions(node_config, registry);
node.start().await?;
```

## Best Practices

### 1. Schema Design
- Use appropriate data types (TEXT for addresses, REAL for decimals, INTEGER for timestamps)
- Always include timestamps for audit trails
- Design for eventual consistency where possible
- Use foreign keys for referential integrity

### 2. Write Operations
- Keep writes atomic and focused
- Validate inputs thoroughly before SQL generation
- Use prepared statements to prevent SQL injection
- Return meaningful error messages

### 3. Triggers
- Keep trigger logic simple and fast
- Avoid recursive triggers
- Use RAISE(ABORT) for validation failures
- Consider performance impact of complex triggers

### 4. Bridge Operations
- Always validate external inputs
- Implement circuit breakers for safety
- Use nonces to prevent replay attacks
- Log all bridge operations for audit

### 5. Performance
- Create appropriate indexes for query patterns
- Use EXPLAIN QUERY PLAN to optimize queries
- Consider materialized views for complex aggregations
- Implement caching for frequently accessed data

## Migration Guide

### From Standalone Application to Extension

If you have an existing application you want to run on SyndDB:

1. **Extract your schema** into SchemaExtension implementations
2. **Convert your API endpoints** to LocalWriteExtension implementations
3. **Move business logic** to TriggerExtension implementations
4. **Implement bridges** if you need external blockchain interaction
5. **Test extensively** with the SyndDB test harness

### Version Upgrades

Extensions support versioning for smooth upgrades:

```rust
impl SchemaExtension for MyExtension {
    fn version(&self) -> u32 { 2 }

    fn migrate_statements(&self, from_version: u32) -> Result<Vec<String>> {
        match from_version {
            1 => Ok(vec![
                "ALTER TABLE my_table ADD COLUMN new_field TEXT".to_string(),
                "UPDATE my_table SET new_field = 'default' WHERE new_field IS NULL".to_string(),
            ]),
            _ => Err(anyhow!("Unsupported migration from version {}", from_version)),
        }
    }
}
```

## Testing Extensions

### Unit Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use synddb_extensions::test_utils::*;

    #[test]
    fn test_schema_creation() {
        let ext = MyExtension::new(test_config());
        let statements = ext.create_statements();

        // Test with in-memory SQLite
        let db = create_test_db();
        for stmt in statements {
            db.execute(&stmt, []).expect("Schema creation failed");
        }
    }

    #[tokio::test]
    async fn test_write_operation() {
        let ext = MyExtension::new(test_config());
        let request = serde_json::json!({
            "field1": "value1",
            "field2": 123
        });

        // Validate
        ext.validate(&request).expect("Validation failed");

        // Convert to SQL
        let statements = ext.to_sql(&request).expect("SQL generation failed");
        assert_eq!(statements.len(), 2);
    }
}
```

### Integration Testing

```rust
#[tokio::test]
async fn test_full_flow() {
    // Start test sequencer with extension
    let sequencer = TestSequencer::with_extension(MyExtension::new(config));
    sequencer.start().await;

    // Submit writes
    let receipt = sequencer.execute_local_write(test_write()).await?;
    assert_eq!(receipt.status, "success");

    // Start replica and verify sync
    let replica = TestReplica::new();
    replica.sync_from(&sequencer).await?;

    // Query and verify state
    let result = replica.query("SELECT * FROM my_table").await?;
    assert_eq!(result.len(), 1);
}
```

## Conclusion

The SyndDB Core + Extensions architecture provides a powerful, type-safe way to build high-performance blockchain applications without dealing with low-level database or replication complexity. By implementing a few trait methods, your extension gains access to the full power of the SyndDB Core:

- Ultra-fast SQLite execution (<1ms latency)
- Automatic state replication across nodes
- Blockchain-backed durability
- Built-in query capabilities
- Optional TEE-secured settlement

The SyndDB Core handles all the infrastructure complexity - extensions focus purely on business logic. This clean separation ensures that:
- Core improvements benefit all extensions automatically
- Extensions can be developed, tested, and deployed independently
- The ecosystem can grow without modifying the Core
- Developers can build powerful applications without deep Core knowledge

Focus on your extension's logic - the SyndDB Core handles everything else.