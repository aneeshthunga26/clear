//! Platform adapters translate native events into Clear's desktop policy.

mod smithay;

pub use smithay::run;
