use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::info;

use prediction_market::app::PredictionMarket;

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
            app.publish()?;
            println!("Created account '{}' with ID {}", name, id);
        }

        Commands::CreateMarket {
            question,
            resolution_time,
            description,
        } => {
            let id = app.create_market(&question, description.as_deref(), resolution_time)?;
            app.publish()?;
            println!("Created market {} - \"{}\"", id, question);
        }

        Commands::Buy {
            account,
            market,
            outcome,
            shares,
        } => {
            let trade = app.buy_shares(account, market, &outcome, shares)?;
            app.publish()?;
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
            app.publish()?;
            println!(
                "Sold {} {} shares in market {} for {} cents",
                trade.shares, trade.outcome, trade.market_id, trade.total
            );
        }

        Commands::Resolve { market, outcome } => {
            app.resolve_market(market, &outcome)?;
            app.publish()?;
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
            app.publish()?;
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
            app.publish()?;
            println!("Processed {} deposits", count);
        }

        Commands::Withdraw {
            account,
            amount,
            destination,
        } => {
            let id = app.request_withdrawal(account, amount, &destination)?;
            app.publish()?;
            println!(
                "Created withdrawal request {} for ${:.2}",
                id,
                amount as f64 / 100.0
            );
        }
    }

    Ok(())
}
