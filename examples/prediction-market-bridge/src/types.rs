//! Domain types for the prediction market.

use serde::{Deserialize, Serialize};

/// Outcome of a binary prediction market.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// The "no" outcome (0).
    No = 0,
    /// The "yes" outcome (1).
    Yes = 1,
}

impl Outcome {
    /// Convert to u8 for contract calls.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Try to convert from u8.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::No),
            1 => Some(Self::Yes),
            _ => None,
        }
    }

    /// Get the opposite outcome.
    pub const fn opposite(self) -> Self {
        match self {
            Self::No => Self::Yes,
            Self::Yes => Self::No,
        }
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::No => write!(f, "no"),
            Self::Yes => write!(f, "yes"),
        }
    }
}

impl std::str::FromStr for Outcome {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "no" | "0" => Ok(Self::No),
            "yes" | "1" => Ok(Self::Yes),
            _ => anyhow::bail!("invalid outcome: {}, expected 'yes' or 'no'", s),
        }
    }
}

/// A binary prediction market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    /// Unique market identifier (hex string).
    pub id: String,
    /// The question being predicted.
    pub question: String,
    /// Unix timestamp when the market can be resolved.
    pub resolution_time: i64,
    /// Whether the market has been resolved.
    pub resolved: bool,
    /// The winning outcome (if resolved).
    pub winning_outcome: Option<Outcome>,
    /// Total YES shares outstanding.
    pub total_yes_shares: i64,
    /// Total NO shares outstanding.
    pub total_no_shares: i64,
    /// Unix timestamp when the market was created.
    pub created_at: i64,
    /// Unix timestamp when the market was resolved (if resolved).
    pub resolved_at: Option<i64>,
}

/// A user's position in a market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    /// Position ID (local).
    pub id: i64,
    /// User's address.
    pub user: String,
    /// Market ID.
    pub market_id: String,
    /// Outcome (YES or NO).
    pub outcome: Outcome,
    /// Number of shares held.
    pub shares: i64,
    /// Total cost basis in cents.
    pub cost_basis: i64,
}

/// A trade record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    /// Trade ID (local).
    pub id: i64,
    /// User's address.
    pub user: String,
    /// Market ID.
    pub market_id: String,
    /// Outcome traded.
    pub outcome: Outcome,
    /// Buy or sell.
    pub side: TradeSide,
    /// Number of shares.
    pub shares: i64,
    /// Price per share in cents.
    pub price: i64,
    /// Total transaction value in cents.
    pub total: i64,
    /// Unix timestamp when executed.
    pub executed_at: i64,
    /// Bridge message ID (if submitted to chain).
    pub message_id: Option<String>,
}

/// Trade side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TradeSide {
    Buy,
    Sell,
}

impl std::fmt::Display for TradeSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buy => write!(f, "buy"),
            Self::Sell => write!(f, "sell"),
        }
    }
}

/// User account with balance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// User's address.
    pub address: String,
    /// Balance in cents.
    pub balance: i64,
    /// Unix timestamp when created.
    pub created_at: i64,
}

/// Constants for the prediction market.
pub mod constants {
    /// Fixed price per share in cents (50/50 pricing).
    pub const SHARE_PRICE: i64 = 50;

    /// Payout per winning share in cents.
    pub const PAYOUT_PER_SHARE: i64 = 100;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outcome_from_str() {
        assert_eq!("yes".parse::<Outcome>().unwrap(), Outcome::Yes);
        assert_eq!("no".parse::<Outcome>().unwrap(), Outcome::No);
        assert_eq!("YES".parse::<Outcome>().unwrap(), Outcome::Yes);
        assert_eq!("1".parse::<Outcome>().unwrap(), Outcome::Yes);
        assert_eq!("0".parse::<Outcome>().unwrap(), Outcome::No);
        assert!("maybe".parse::<Outcome>().is_err());
    }

    #[test]
    fn test_outcome_opposite() {
        assert_eq!(Outcome::Yes.opposite(), Outcome::No);
        assert_eq!(Outcome::No.opposite(), Outcome::Yes);
    }
}
