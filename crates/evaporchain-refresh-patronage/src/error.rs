use evaporchain_energy_kernel::refresh_pool::RefreshPoolError;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CovenantError {
    #[error("object {object_id:?} already has an active covenant")]
    AlreadyPledged { object_id: Vec<u8> },
    #[error("no active covenant for object {0:?}")]
    UnknownCovenant(Vec<u8>),
    #[error("covenant for {object_id:?} expired at epoch {expires}")]
    Expired { object_id: Vec<u8>, expires: u64 },
    #[error("donation_per_epoch and epochs must both be non-zero")]
    ZeroPledge,
    #[error("refresh pool error: {0}")]
    Pool(#[from] RefreshPoolError),
}
