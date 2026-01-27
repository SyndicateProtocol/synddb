//! Custom validation rules for the price oracle
//!
//! This module contains application-specific validation rules that extend
//! the base synddb-validator functionality.

mod price_consistency;

pub use price_consistency::PriceConsistencyRule;
