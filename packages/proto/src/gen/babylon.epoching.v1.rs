// @generated
/// Epoch is a structure that contains the metadata of an epoch
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Epoch {
    /// epoch_number is the number of this epoch
    #[prost(uint64, tag="1")]
    pub epoch_number: u64,
    /// current_epoch_interval is the epoch interval at the time of this epoch
    #[prost(uint64, tag="2")]
    pub current_epoch_interval: u64,
    /// first_block_height is the height of the first block in this epoch
    #[prost(uint64, tag="3")]
    pub first_block_height: u64,
    /// last_block_time is the time of the last block in this epoch.
    /// Babylon needs to remember the last header's time of each epoch to complete
    /// unbonding validators/delegations when a previous epoch's checkpoint is
    /// finalised. The last_block_time field is nil in the epoch's beginning, and
    /// is set upon the end of this epoch.
    #[prost(message, optional, tag="4")]
    pub last_block_time: ::core::option::Option<::pbjson_types::Timestamp>,
    /// sealer is the last block of the sealed epoch
    /// sealer_app_hash points to the sealer but stored in the 1st header
    /// of the next epoch
    #[prost(bytes="bytes", tag="5")]
    pub sealer_app_hash: ::prost::bytes::Bytes,
    /// sealer_block_hash is the hash of the sealer
    /// the validator set has generated a BLS multisig on the hash,
    /// i.e., hash of the last block in the epoch
    #[prost(bytes="bytes", tag="6")]
    pub sealer_block_hash: ::prost::bytes::Bytes,
}
// @@protoc_insertion_point(module)
