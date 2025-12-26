use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::info;

use prediction_market::app::PredictionMarket;

#[cfg(feature = "chain-monitor")]
use {
    crossbeam_channel::unbounded,
    prediction_market::chain_monitor::{confirm_withdrawal, insert_deposit, BridgeEventHandler},
    std::sync::Arc,
    synddb_chain_monitor::{config::ChainMonitorConfig, monitor::ChainMonitor},
    tracing::warn,
};

#[derive(Parser)]
#[command(name = "prediction-market")]
#[command(about = "Example prediction market using SyndDB", long_about = None)]
struct Cli {
    /// Path to the `SQLite` database
    #[arg(short, long, default_value = "market.db")]
    db: PathBuf,

    /// Sequencer URL (enables `SyndDB` replication)
    #[arg(long, env = "SEQUENCER_URL")]
    sequencer: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize the database schema
    Init,

    /// Create a user account
    CreateAccount {
        /// Account name (unique identifier)
        name: String,
    },

    /// Create a new prediction market
    CreateMarket {
        /// The question being predicted
        question: String,

        /// Resolution time (Unix timestamp)
        #[arg(long)]
        resolution_time: i64,

        /// Optional description
        #[arg(long)]
        description: Option<String>,
    },

    /// Buy shares in a market
    Buy {
        /// Account ID
        #[arg(long)]
        account: i64,

        /// Market ID
        #[arg(long)]
        market: i64,

        /// Outcome to buy: "yes" or "no"
        #[arg(long)]
        outcome: String,

        /// Number of shares to buy
        #[arg(long)]
        shares: i64,
    },

    /// Sell shares in a market
    Sell {
        /// Account ID
        #[arg(long)]
        account: i64,

        /// Market ID
        #[arg(long)]
        market: i64,

        /// Outcome to sell: "yes" or "no"
        #[arg(long)]
        outcome: String,

        /// Number of shares to sell
        #[arg(long)]
        shares: i64,
    },

    /// Resolve a market with an outcome
    Resolve {
        /// Market ID
        #[arg(long)]
        market: i64,

        /// Outcome: "yes" or "no"
        #[arg(long)]
        outcome: String,
    },

    /// Show status of markets and accounts
    Status,

    /// Simulate a deposit from L1 (for testing)
    ///
    /// In production, the chain monitor would insert deposit records when it
    /// sees `Deposit` events from the bridge contract.
    SimulateDeposit {
        /// Transaction hash
        #[arg(long)]
        tx_hash: String,

        /// L1 sender address
        #[arg(long)]
        from: String,

        /// L2 destination address (becomes the account name)
        #[arg(long)]
        to: String,

        /// Amount in cents
        #[arg(long)]
        amount: i64,

        /// Block number
        #[arg(long, default_value = "1")]
        block: i64,
    },

    /// Process pending deposits
    ProcessDeposits,

    /// Request a withdrawal to L1
    Withdraw {
        /// Account ID
        #[arg(long)]
        account: i64,

        /// Amount in cents
        #[arg(long)]
        amount: i64,

        /// Destination address on L1
        #[arg(long)]
        destination: String,
    },

    /// Watch the bridge contract for deposit/withdrawal events
    #[cfg(feature = "chain-monitor")]
    Watch {
        /// WebSocket RPC URL for the L1 chain
        #[arg(long, env = "WS_URL")]
        ws_url: String,

        /// Bridge contract address to monitor
        #[arg(long, env = "BRIDGE_CONTRACT")]
        bridge: String,

        /// Block number to start watching from
        #[arg(long, default_value = "1")]
        start_block: u64,
    },

    /// Run the HTTP server
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    let cli = Cli::parse();

    let db_path = cli.db.to_string_lossy();
    let app = PredictionMarket::new(&db_path, cli.sequencer.as_deref())?;

    if app.is_replicated() {
        info!("SyndDB replication: ENABLED");
    } else {
        info!("SyndDB replication: DISABLED (no sequencer URL)");
    }

    match cli.command {
        Commands::Init => {
            info!("Database initialized at {:?}", cli.db);
        }

        Commands::CreateAccount { name } => {
            let id = app.create_account(&name)?;
            app.publish_changeset()?;
            println!("Created account '{}' with ID {}", name, id);
        }

        Commands::CreateMarket {
            question,
            resolution_time,
            description,
        } => {
            let id = app.create_market(&question, description.as_deref(), resolution_time)?;
            app.publish_changeset()?;
            println!("Created market {} - \"{}\"", id, question);
        }

        Commands::Buy {
            account,
            market,
            outcome,
            shares,
        } => {
            let trade = app.buy_shares(account, market, &outcome, shares)?;
            app.publish_changeset()?;
            println!(
                "Bought {} {} shares in market {} for {} cents",
                trade.shares, trade.outcome, trade.market_id, trade.total
            );
        }

        Commands::Sell {
            account,
            market,
            outcome,
            shares,
        } => {
            let trade = app.sell_shares(account, market, &outcome, shares)?;
            app.publish_changeset()?;
            println!(
                "Sold {} {} shares in market {} for {} cents",
                trade.shares, trade.outcome, trade.market_id, trade.total
            );
        }

        Commands::Resolve { market, outcome } => {
            app.resolve_market(market, &outcome)?;
            app.publish_changeset()?;
            println!("Resolved market {} as '{}'", market, outcome);
        }

        Commands::Status => {
            println!("\n=== Accounts ===");
            for account in app.list_accounts()? {
                println!(
                    "  [{}] {} - ${:.2}",
                    account.id,
                    account.name,
                    account.balance as f64 / 100.0
                );

                let positions = app.get_positions(account.id)?;
                for pos in positions {
                    println!(
                        "       Market {}: {} {} shares",
                        pos.market_id, pos.shares, pos.outcome
                    );
                }
            }

            println!("\n=== Markets ===");
            for market in app.list_markets()? {
                let status = if market.outcome == "unresolved" {
                    "OPEN".to_string()
                } else {
                    format!("RESOLVED: {}", market.outcome)
                };
                println!("  [{}] {} - {}", market.id, market.question, status);
            }

            let withdrawals = app.list_pending_withdrawals()?;
            if !withdrawals.is_empty() {
                println!("\n=== Pending Withdrawals ===");
                for w in withdrawals {
                    println!(
                        "  [{}] {} - ${:.2} to {}",
                        w.id,
                        w.account_name,
                        w.amount as f64 / 100.0,
                        w.destination_address
                    );
                }
            }
        }

        Commands::SimulateDeposit {
            tx_hash,
            from,
            to,
            amount,
            block,
        } => {
            let id = app.simulate_deposit(&tx_hash, &from, &to, amount, block)?;
            app.publish_changeset()?;
            println!(
                "Simulated deposit {} for ${:.2} from {} to {}",
                id,
                amount as f64 / 100.0,
                from,
                to
            );
        }

        Commands::ProcessDeposits => {
            let count = app.process_deposits()?;
            app.publish_changeset()?;
            println!("Processed {} deposits", count);
        }

        Commands::Withdraw {
            account,
            amount,
            destination,
        } => {
            let id = app.request_withdrawal(account, amount, &destination)?;
            app.publish_changeset()?;
            println!(
                "Created withdrawal request {} for ${:.2}",
                id,
                amount as f64 / 100.0
            );
        }

        #[cfg(feature = "chain-monitor")]
        Commands::Watch {
            ws_url,
            bridge,
            start_block,
        } => {
            run_chain_monitor(&app, &ws_url, &bridge, start_block)?;
        }

        Commands::Serve { port } => {
            // Note: We don't use the 'app' created above - the HTTP server manages its own connections
            drop(app);
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(prediction_market::http::serve(
                db_path.to_string(),
                cli.sequencer.clone(),
                port,
            ))?;
        }
    }

    Ok(())
}

/// Run the chain monitor to watch for bridge events
#[cfg(feature = "chain-monitor")]
fn run_chain_monitor(
    app: &PredictionMarket,
    ws_url: &str,
    bridge_address: &str,
    start_block: u64,
) -> Result<()> {
    use alloy::primitives::Address;
    use url::Url;

    info!("Starting chain monitor...");
    info!("  WebSocket URL: {}", ws_url);
    info!("  Bridge address: {}", bridge_address);
    info!("  Start block: {}", start_block);

    // Parse the bridge address
    let contract_address: Address = bridge_address
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid bridge address: {}", e))?;

    // Parse the WebSocket URL
    let ws_url = Url::parse(ws_url).map_err(|e| anyhow::anyhow!("Invalid WebSocket URL: {}", e))?;

    // Create channels for deposit and withdrawal events
    let (deposit_tx, deposit_rx) = unbounded();
    let (withdrawal_tx, withdrawal_rx) = unbounded();

    // Create composite handler for both event types
    let handler = Arc::new(BridgeEventHandler::new(deposit_tx, withdrawal_tx));

    // Create chain monitor config
    let db_path = app.conn().path().unwrap_or("market.db").to_string();
    let config = ChainMonitorConfig::new(vec![ws_url], contract_address, start_block)
        .with_event_store_path(format!("{}.chain_events.db", db_path));

    // Run the monitor - we need to process events in a separate thread since
    // rusqlite::Connection is not Sync and ChainMonitor::run() blocks
    let rt = tokio::runtime::Runtime::new()?;

    // Open a separate connection for the db processing thread
    // (rusqlite::Connection is not Send/Sync, so we can't share the app's connection)
    let db_path_for_thread = db_path;
    let db_thread = std::thread::spawn(move || {
        let conn = match rusqlite::Connection::open(&db_path_for_thread) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to open database for chain monitor: {}", e);
                return;
            }
        };

        loop {
            // Process any pending deposits
            match deposit_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(deposit) => {
                    info!(
                        tx_hash = %deposit.tx_hash,
                        from = %deposit.from_address,
                        to = %deposit.to_address,
                        amount = deposit.amount,
                        "Inserting deposit into database"
                    );

                    if let Err(e) = insert_deposit(&conn, &deposit) {
                        warn!("Failed to insert deposit: {}", e);
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }

            // Process any pending withdrawal confirmations
            match withdrawal_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(confirmation) => {
                    info!(
                        tx_hash = %confirmation.tx_hash,
                        recipient = %confirmation.recipient_address,
                        amount = confirmation.amount,
                        "Confirming withdrawal in database"
                    );

                    match confirm_withdrawal(&conn, &confirmation) {
                        Ok(rows) => {
                            if rows > 0 {
                                info!("Confirmed {} withdrawal(s)", rows);
                            } else {
                                warn!("No matching withdrawal found to confirm");
                            }
                        }
                        Err(e) => {
                            warn!("Failed to confirm withdrawal: {}", e);
                        }
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    rt.block_on(async {
        // Create chain monitor (async)
        let mut monitor = match ChainMonitor::new(config, handler).await {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("Failed to create chain monitor: {}", e);
                return;
            }
        };

        info!("Chain monitor started. Press Ctrl+C to stop.");

        // Run the monitor until it errors or is interrupted
        tokio::select! {
            result = monitor.run() => {
                if let Err(e) = result {
                    tracing::error!("Chain monitor error: {}", e);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Received shutdown signal");
            }
        }
    });

    // When the monitor is dropped, the handler (and its senders) are dropped,
    // causing the receivers in the db thread to disconnect and exit
    let _ = db_thread.join();

    info!("Chain monitor stopped");
    Ok(())
}
