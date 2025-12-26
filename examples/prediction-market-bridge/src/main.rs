//! Binary Prediction Market CLI using Message-Passing Bridge Paradigm
//!
//! This example demonstrates the ergonomic differences between message-passing
//! and direct SQLite operations for a prediction market application.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;

use prediction_market_bridge::{
    bridge::BridgeClient,
    schema,
    store::Store,
    Outcome,
};

#[derive(Parser)]
#[command(name = "prediction-market-bridge")]
#[command(about = "Binary prediction market using message-passing Bridge paradigm")]
struct Cli {
    /// Path to the SQLite database (local cache)
    #[arg(short, long, default_value = "market-cache.db")]
    db: PathBuf,

    /// Bridge validator URL
    #[arg(long, env = "BRIDGE_VALIDATOR_URL", default_value = "http://localhost:8080")]
    validator_url: String,

    /// Application domain (32 bytes hex)
    #[arg(long, env = "BRIDGE_DOMAIN")]
    domain: Option<String>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize the local cache database
    Init,

    /// Create a new prediction market
    CreateMarket {
        /// Unique market ID (hex string, will be padded to 32 bytes)
        #[arg(long)]
        id: String,

        /// The question being predicted
        #[arg(long)]
        question: String,

        /// Resolution time (Unix timestamp)
        #[arg(long)]
        resolution_time: i64,
    },

    /// Deposit funds for a user
    Deposit {
        /// User address
        #[arg(long)]
        user: String,

        /// Amount in cents
        #[arg(long)]
        amount: u64,
    },

    /// Buy shares in a market
    Buy {
        /// Market ID
        #[arg(long)]
        market: String,

        /// User address
        #[arg(long)]
        user: String,

        /// Outcome: "yes" or "no"
        #[arg(long)]
        outcome: String,

        /// Number of shares
        #[arg(long)]
        shares: u64,
    },

    /// Sell shares in a market
    Sell {
        /// Market ID
        #[arg(long)]
        market: String,

        /// User address
        #[arg(long)]
        user: String,

        /// Outcome: "yes" or "no"
        #[arg(long)]
        outcome: String,

        /// Number of shares
        #[arg(long)]
        shares: u64,
    },

    /// Resolve a market
    Resolve {
        /// Market ID
        #[arg(long)]
        market: String,

        /// Winning outcome: "yes" or "no"
        #[arg(long)]
        outcome: String,
    },

    /// List markets (from local cache)
    Markets,

    /// Show market details (from local cache)
    Market {
        /// Market ID
        id: String,
    },

    /// Show user positions (from local cache)
    Positions {
        /// User address
        user: String,
    },

    /// Show portfolio summary (from local cache)
    Portfolio {
        /// User address
        user: String,
    },

    /// Show leaderboard (from local cache)
    Leaderboard {
        /// Number of entries
        #[arg(long, default_value = "10")]
        limit: usize,
    },

    /// Check message status
    Status {
        /// Message ID
        message_id: String,
    },

    /// Run the HTTP server
    Serve {
        /// Port to listen on
        #[arg(long, default_value = "3000")]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let filter = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    // Open the local cache
    let store = if cli.db.to_string_lossy() == ":memory:" {
        Store::open_in_memory()?
    } else {
        Store::open(cli.db.to_str().unwrap())?
    };

    // Create Bridge client if domain is provided
    let bridge_client = cli.domain.as_ref().map(|domain| {
        BridgeClient::new(
            &cli.validator_url,
            domain,
            3, // max retries
            Duration::from_secs(1), // retry delay
        )
    });

    match cli.command {
        Commands::Init => {
            info!("Initializing local cache database...");
            schema::initialize_schema(store.conn())?;
            info!("Database initialized at {:?}", cli.db);
        }

        Commands::CreateMarket {
            id,
            question,
            resolution_time,
        } => {
            let client = bridge_client.ok_or_else(|| anyhow::anyhow!("--domain required for Bridge operations"))?;

            // Pad ID to 32 bytes if needed
            let padded_id = pad_market_id(&id)?;

            info!("Submitting createMarket to Bridge...");
            let result = client.create_market(&padded_id, &question, resolution_time as u64).await?;

            if result.success {
                info!("Market creation submitted!");
                let msg_id = result.message_id.clone().unwrap_or_default();
                info!("  Message ID: {}", msg_id);
                info!("  Market ID: {}", padded_id);

                // Wait for completion
                if !msg_id.is_empty() {
                    info!("Waiting for on-chain confirmation...");
                    let status = client
                        .wait_for_completion(&msg_id, Duration::from_secs(60), Duration::from_secs(2))
                        .await?;

                    if status.is_success() {
                        info!("Market created on-chain!");
                        if let Some(tx) = status.tx_hash {
                            info!("  Transaction: {}", tx);
                        }
                    } else {
                        anyhow::bail!("Market creation failed: {}", status.status);
                    }
                }
            } else {
                anyhow::bail!(
                    "Market creation rejected: {} - {}",
                    result.error_code.unwrap_or_default(),
                    result.error_message.unwrap_or_default()
                );
            }
        }

        Commands::Deposit { user, amount } => {
            let client = bridge_client.ok_or_else(|| anyhow::anyhow!("--domain required for Bridge operations"))?;

            info!("Submitting deposit to Bridge...");
            let result = client.deposit(&user, amount).await?;

            if result.success {
                info!("Deposit submitted!");
                info!("  Message ID: {}", result.message_id.unwrap_or_default());
            } else {
                anyhow::bail!(
                    "Deposit rejected: {} - {}",
                    result.error_code.unwrap_or_default(),
                    result.error_message.unwrap_or_default()
                );
            }
        }

        Commands::Buy {
            market,
            user,
            outcome,
            shares,
        } => {
            let client = bridge_client.ok_or_else(|| anyhow::anyhow!("--domain required for Bridge operations"))?;
            let outcome: Outcome = outcome.parse()?;
            let padded_id = pad_market_id(&market)?;

            info!("Submitting buyShares to Bridge...");
            let result = client.buy_shares(&padded_id, &user, outcome, shares).await?;

            if result.success {
                info!("Buy order submitted!");
                let msg_id = result.message_id.clone().unwrap_or_default();
                info!("  Message ID: {}", msg_id);
                info!("  {} shares of {} @ 50 cents = {} cents", shares, outcome, shares * 50);

                // Wait for completion
                if !msg_id.is_empty() {
                    info!("Waiting for on-chain confirmation...");
                    let status = client
                        .wait_for_completion(&msg_id, Duration::from_secs(60), Duration::from_secs(2))
                        .await?;

                    if status.is_success() {
                        info!("Trade confirmed on-chain!");
                    } else {
                        anyhow::bail!("Trade failed: {}", status.status);
                    }
                }
            } else {
                anyhow::bail!(
                    "Buy rejected: {} - {}",
                    result.error_code.unwrap_or_default(),
                    result.error_message.unwrap_or_default()
                );
            }
        }

        Commands::Sell {
            market,
            user,
            outcome,
            shares,
        } => {
            let client = bridge_client.ok_or_else(|| anyhow::anyhow!("--domain required for Bridge operations"))?;
            let outcome: Outcome = outcome.parse()?;
            let padded_id = pad_market_id(&market)?;

            info!("Submitting sellShares to Bridge...");
            let result = client.sell_shares(&padded_id, &user, outcome, shares).await?;

            if result.success {
                info!("Sell order submitted!");
                info!("  Message ID: {}", result.message_id.unwrap_or_default());
            } else {
                anyhow::bail!(
                    "Sell rejected: {} - {}",
                    result.error_code.unwrap_or_default(),
                    result.error_message.unwrap_or_default()
                );
            }
        }

        Commands::Resolve { market, outcome } => {
            let client = bridge_client.ok_or_else(|| anyhow::anyhow!("--domain required for Bridge operations"))?;
            let outcome: Outcome = outcome.parse()?;
            let padded_id = pad_market_id(&market)?;

            info!("Submitting resolveMarket to Bridge...");
            let result = client.resolve_market(&padded_id, outcome).await?;

            if result.success {
                info!("Resolution submitted!");
                info!("  Message ID: {}", result.message_id.unwrap_or_default());
                info!("  Winning outcome: {}", outcome);
            } else {
                anyhow::bail!(
                    "Resolution rejected: {} - {}",
                    result.error_code.unwrap_or_default(),
                    result.error_message.unwrap_or_default()
                );
            }
        }

        Commands::Markets => {
            let markets = store.list_markets()?;
            if markets.is_empty() {
                info!("No markets in local cache");
            } else {
                info!("Markets ({}):", markets.len());
                for market in markets {
                    let status = if market.resolved {
                        format!("resolved: {}", market.winning_outcome.map_or("?".to_string(), |o| o.to_string()))
                    } else {
                        "active".to_string()
                    };
                    info!(
                        "  {} - {} [{}]",
                        &market.id[..10],
                        market.question,
                        status
                    );
                }
            }
        }

        Commands::Market { id } => {
            let padded_id = pad_market_id(&id)?;
            match store.get_market(&padded_id)? {
                Some(market) => {
                    info!("Market: {}", market.id);
                    info!("  Question: {}", market.question);
                    info!("  Resolution time: {}", market.resolution_time);
                    info!("  Resolved: {}", market.resolved);
                    if let Some(outcome) = market.winning_outcome {
                        info!("  Winning outcome: {}", outcome);
                    }
                    info!("  YES shares: {}", market.total_yes_shares);
                    info!("  NO shares: {}", market.total_no_shares);

                    if let Some(stats) = store.get_market_stats(&padded_id)? {
                        let total = stats.yes_shares + stats.no_shares;
                        if total > 0 {
                            info!("  YES implied prob: {:.1}%", stats.yes_percentage);
                        }
                        info!("  Total volume: {} cents", stats.total_volume);
                        info!("  Unique traders: {}", stats.unique_traders);
                    }
                }
                None => {
                    info!("Market not found in local cache");
                }
            }
        }

        Commands::Positions { user } => {
            let positions = store.get_user_positions(&user)?;
            if positions.is_empty() {
                info!("No positions for {}", user);
            } else {
                info!("Positions for {}:", user);
                for pos in positions {
                    info!(
                        "  {} {}: {} shares (cost: {} cents)",
                        &pos.market_id[..10],
                        pos.outcome,
                        pos.shares,
                        pos.cost_basis
                    );
                }
            }
        }

        Commands::Portfolio { user } => {
            let summary = store.get_portfolio_value(&user)?;
            let realized = store.get_realized_pnl(&user)?;

            info!("Portfolio for {}:", user);
            info!("  Active positions: {}", summary.total_positions);
            info!("  Total cost basis: {} cents", summary.total_cost_basis);
            info!("  Estimated value: {} cents", summary.estimated_value);
            info!("  Unrealized P&L: {} cents", summary.unrealized_pnl);
            info!("  Realized P&L: {} cents", realized);
        }

        Commands::Leaderboard { limit } => {
            let entries = store.get_leaderboard(limit)?;
            if entries.is_empty() {
                info!("No resolved trades yet");
            } else {
                info!("Leaderboard (top {} by realized P&L):", limit);
                for (i, entry) in entries.iter().enumerate() {
                    info!(
                        "  {}. {} - P&L: {} cents ({} markets)",
                        i + 1,
                        &entry.user[..10],
                        entry.total_pnl,
                        entry.markets_traded
                    );
                }
            }
        }

        Commands::Status { message_id } => {
            let client = bridge_client.ok_or_else(|| anyhow::anyhow!("--domain required for Bridge operations"))?;

            let status = client.get_message_status(&message_id).await?;
            info!("Message: {}", status.message_id);
            info!("  Stage: {} ({})", status.stage, status.status);
            info!("  Executed: {}", status.executed);
            info!(
                "  Signatures: {}/{}",
                status.signatures_collected, status.signature_threshold
            );
            if let Some(tx) = status.tx_hash {
                info!("  Transaction: {}", tx);
            }
            if let Some(block) = status.block_number {
                info!("  Block: {}", block);
            }
        }

        Commands::Serve { port } => {
            info!("HTTP server not yet implemented");
            info!("Would serve on port {}", port);
            // TODO: Implement HTTP server
        }
    }

    Ok(())
}

/// Pad a market ID to 32 bytes (64 hex chars).
fn pad_market_id(id: &str) -> Result<String> {
    let id = id.strip_prefix("0x").unwrap_or(id);

    if id.len() > 64 {
        anyhow::bail!("Market ID too long (max 32 bytes)");
    }

    // Left-pad with zeros
    let padded = format!("{:0>64}", id);
    Ok(format!("0x{}", padded))
}
