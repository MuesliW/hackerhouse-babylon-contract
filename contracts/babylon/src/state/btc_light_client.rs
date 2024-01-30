//! btc_light_client is the storage for the BTC header chain
use babylon_bitcoin::BlockHeader;
use prost::Message;

use cosmwasm_std::{StdResult, Storage};
use cw_storage_plus::{Item, Map};

use babylon_proto::babylon::btclightclient::v1::BtcHeaderInfo;

use crate::error::BTCLightclientError;
use crate::state::config::CONFIG;
use crate::utils::btc_light_client::{total_work, verify_headers};

pub const BTC_HEADERS: Map<&[u8], Vec<u8>> = Map::new("btc_lc_headers");
pub const BTC_HEADER_BASE: Item<Vec<u8>> = Item::new("btc_lc_header_base");
pub const BTC_HEIGHTS: Map<&[u8], u64> = Map::new("btc_lc_heights");
pub const BTC_TIP: Item<Vec<u8>> = Item::new("btc_lc_tip");

// getters for storages

// is_initialized checks if the BTC light client has been initialised or not
// the check is done by checking existence of base header
pub fn is_initialized(storage: &mut dyn Storage) -> bool {
    BTC_HEADER_BASE.load(storage).is_ok()
}

// getter/setter for base header
pub fn get_base_header(storage: &mut dyn Storage) -> Result<BtcHeaderInfo, BTCLightclientError> {
    // NOTE: if init is successful, then base header is guaranteed to be in storage and decodable
    let base_header_bytes = BTC_HEADER_BASE.load(storage)?;
    BtcHeaderInfo::decode(base_header_bytes.as_slice()).map_err(BTCLightclientError::DecodeError)
}

fn set_base_header(storage: &mut dyn Storage, base_header: &BtcHeaderInfo) -> StdResult<()> {
    let base_header_bytes = base_header.encode_to_vec();
    BTC_HEADER_BASE.save(storage, &base_header_bytes)
}

// getter/setter for chain tip
pub fn get_tip(storage: &mut dyn Storage) -> Result<BtcHeaderInfo, BTCLightclientError> {
    let tip_bytes = BTC_TIP.load(storage)?;
    // NOTE: if init is successful, then tip header is guaranteed to be correct
    BtcHeaderInfo::decode(tip_bytes.as_slice()).map_err(BTCLightclientError::DecodeError)
}

fn set_tip(storage: &mut dyn Storage, tip: &BtcHeaderInfo) -> StdResult<()> {
    let tip_bytes = &tip.encode_to_vec();
    BTC_TIP.save(storage, tip_bytes)
}

// insert_headers inserts BTC headers that have passed the
// verification to the header chain storages, including
// - insert all headers
// - insert all hash-to-height indices
fn insert_headers(storage: &mut dyn Storage, new_headers: &[BtcHeaderInfo]) -> StdResult<()> {
    // Add all the headers by hash
    for new_header in new_headers.iter() {
        // insert header
        let hash_bytes: &[u8] = new_header.hash.as_ref();
        let header_bytes = new_header.encode_to_vec();
        BTC_HEADERS.save(storage, hash_bytes, &header_bytes)?;
        BTC_HEIGHTS.save(storage, hash_bytes, &new_header.height)?;
    }
    Ok(())
}

// remove_headers removes BTC headers from the header chain storages, including
// - remove all headers from a fork, starting from the fork's tip
// - remove all hash-to-height indices
fn remove_headers(
    storage: &mut dyn Storage,
    tip_header: &BtcHeaderInfo,
    parent_header: &BtcHeaderInfo,
) -> Result<(), BTCLightclientError> {
    // Remove all the headers by hash starting from the tip, until hitting the parent header
    let mut rem_header = tip_header.clone();
    while rem_header.hash != parent_header.hash {
        // Remove header from storage
        BTC_HEADERS.remove(storage, &rem_header.hash);
        BTC_HEIGHTS.remove(storage, &rem_header.hash);
        // Decode BTC header to get prev header hash
        let rem_btc_header: BlockHeader = babylon_bitcoin::deserialize(rem_header.header.as_ref())
            .map_err(|_| BTCLightclientError::BTCHeaderDecodeError {})?;
        rem_header = get_header(storage, rem_btc_header.prev_blockhash.as_ref())?;
    }
    Ok(())
}

// get_header retrieves the BTC header of a given hash
pub fn get_header(
    storage: &mut dyn Storage,
    hash: &[u8],
) -> Result<BtcHeaderInfo, BTCLightclientError> {
    // Try to find the header with the given hash
    let header_bytes = BTC_HEADERS.load(storage, hash).map_err(|_| {
        BTCLightclientError::BTCHeaderNotFoundError {
            hash: hex::encode(hash),
        }
    })?;

    // Try to decode the header
    let header = BtcHeaderInfo::decode(header_bytes.as_slice())
        .map_err(|_| BTCLightclientError::BTCHeaderDecodeError {})?;

    Ok(header)
}

/// init initialises the BTC header chain storage
/// It takes BTC headers between
/// - the BTC tip upon the last finalised epoch
/// - the current tip
pub fn init(
    storage: &mut dyn Storage,
    headers: &[BtcHeaderInfo],
) -> Result<(), BTCLightclientError> {
    let cfg = CONFIG.load(storage)?;
    let btc_network = babylon_bitcoin::chain_params::get_chain_params(cfg.network);

    // ensure there are >=w+1 headers, i.e., a base header and at least w subsequent
    // ones as a w-deep proof
    if (headers.len() as u64) < cfg.checkpoint_finalization_timeout + 1 {
        return Err(BTCLightclientError::InitError {});
    }

    // base header is the first header in the list
    let base_header = headers.first().ok_or(BTCLightclientError::InitError {})?;

    // decode this header to rust-bitcoin's type
    let base_btc_header: BlockHeader = babylon_bitcoin::deserialize(base_header.header.as_ref())
        .map_err(|_| BTCLightclientError::BTCHeaderDecodeError {})?;

    // verify the base header's pow
    if babylon_bitcoin::pow::verify_header_pow(&btc_network, &base_btc_header).is_err() {
        return Err(BTCLightclientError::BTCHeaderError {});
    }

    // verify subsequent headers
    let new_headers = &headers[1..headers.len()];
    verify_headers(&btc_network, base_header, new_headers)?;

    // all good, set base header, insert all headers, and set tip

    // initialise base header
    // NOTE: not changeable in the future
    set_base_header(storage, base_header)?;
    // insert all headers
    insert_headers(storage, headers)?;
    // set tip header
    set_tip(
        storage,
        headers.last().ok_or(BTCLightclientError::InitError {})?,
    )?;
    Ok(())
}

/// handle_btc_headers_from_babylon verifies and inserts a number of
/// finalised BTC headers to the header chain storage, and update
/// the chain tip.
///
/// NOTE: upon each finalised epoch e, Babylon will send BTC headers between
/// - the common ancestor of
///   - BTC tip upon finalising epoch e-1
///   - BTC tip upon finalising epoch e,
/// - BTC tip upon finalising epoch e
/// such that Babylon contract maintains the same canonical BTC header chain
/// as Babylon.
pub fn handle_btc_headers_from_babylon(
    storage: &mut dyn Storage,
    new_headers: &[BtcHeaderInfo],
) -> Result<(), BTCLightclientError> {
    let cfg = CONFIG.load(storage)?;
    let btc_network = babylon_bitcoin::chain_params::get_chain_params(cfg.network);

    let cur_tip = get_tip(storage)?;
    let cur_tip_hash = cur_tip.hash.clone();

    // decode the first header in these new headers
    let first_new_header = new_headers
        .first()
        .ok_or(BTCLightclientError::BTCHeaderEmpty {})?;
    let first_new_btc_header: BlockHeader =
        babylon_bitcoin::deserialize(first_new_header.header.as_ref())
            .map_err(|_| BTCLightclientError::BTCHeaderDecodeError {})?;

    if first_new_btc_header.prev_blockhash.as_ref() == cur_tip_hash {
        // Most common case: extending the current tip

        // Verify each new header after `current_tip` iteratively
        verify_headers(&btc_network, &cur_tip.clone(), new_headers)?;

        // All good, add all the headers to the BTC light client store
        insert_headers(storage, new_headers)?;

        // Update tip
        let new_tip = new_headers
            .last()
            .ok_or(BTCLightclientError::BTCHeaderEmpty {})?;
        set_tip(storage, new_tip)?;
    } else {
        // Here we received a potential new fork
        let parent_hash = first_new_btc_header.prev_blockhash.as_ref();
        let fork_parent = get_header(storage, parent_hash)?;

        // Verify each new header after `fork_parent` iteratively
        verify_headers(&btc_network, &fork_parent, new_headers)?;

        let new_tip = new_headers
            .last()
            .ok_or(BTCLightclientError::BTCHeaderEmpty {})?;

        let new_tip_work = total_work(new_tip)?;
        let cur_tip_work = total_work(&cur_tip)?;
        if new_tip_work <= cur_tip_work {
            return Err(BTCLightclientError::BTCChainWithNotEnoughWork(
                new_tip_work,
                cur_tip_work,
            ));
        }

        // All good, add all the headers to the BTC light client store
        insert_headers(storage, new_headers)?;

        // Update tip
        set_tip(storage, new_tip)?;

        // Remove all headers from the old fork
        remove_headers(storage, &cur_tip, &fork_parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use babylon_proto::babylon::btclightclient::v1::{BtcHeaderInfo, QueryMainChainResponse};
    use cosmwasm_std::testing::mock_dependencies;
    use std::fs;

    const TESTDATA: &str = "../../testdata/btc_light_client.dat";

    fn get_test_headers() -> Vec<BtcHeaderInfo> {
        let testdata: &[u8] = &fs::read(TESTDATA).unwrap();
        let resp = QueryMainChainResponse::decode(testdata).unwrap();
        resp.headers
    }

    // btc_lc_works simulates initialisation of BTC light client storage, then insertion of
    // a number of headers. It ensures that the correctness of initialisation/insertion upon
    // a list of correct BTC headers on Bitcoin regtest net.
    #[test]
    fn btc_lc_works() {
        let test_headers_vec = get_test_headers();
        let test_headers = test_headers_vec.as_slice();
        let deps = mock_dependencies();
        let mut storage = deps.storage;

        // set config first
        let w = 2_usize;
        let cfg = super::super::config::Config {
            network: babylon_bitcoin::chain_params::Network::Regtest,
            babylon_tag: vec![0x1, 0x2, 0x3, 0x4],
            btc_confirmation_depth: 1,
            checkpoint_finalization_timeout: w as u64,
            notify_cosmos_zone: false,
        };
        CONFIG.save(&mut storage, &cfg).unwrap();

        // testing initialisation with w+1 headers
        let test_init_headers: &[BtcHeaderInfo] = &test_headers[0..w + 1];
        init(&mut storage, test_init_headers).unwrap();

        // ensure tip is set
        let tip_expected = test_init_headers.last().unwrap();
        let tip_actual = get_tip(&mut storage).unwrap();
        assert_eq!(*tip_expected, tip_actual);
        // ensure base header is set
        let base_expected = test_init_headers.first().unwrap();
        let base_actual = get_base_header(&mut storage).unwrap();
        assert_eq!(*base_expected, base_actual);
        // ensure all headers are correctly inserted
        for header_expected in test_init_headers.iter() {
            let init_header_actual =
                get_header(&mut storage, header_expected.hash.as_ref()).unwrap();
            assert_eq!(*header_expected, init_header_actual);

            let actual_height = BTC_HEIGHTS
                .load(&storage, header_expected.hash.as_ref())
                .unwrap();
            assert_eq!(header_expected.height, actual_height);
        }

        // handling subsequent headers
        let test_new_headers = &test_headers[w + 1..test_headers.len()];
        handle_btc_headers_from_babylon(&mut storage, test_new_headers).unwrap();

        // ensure tip is set
        let tip_expected = test_headers.last().unwrap();
        let tip_actual = get_tip(&mut storage).unwrap();
        assert_eq!(*tip_expected, tip_actual);
        // ensure all headers are correctly inserted
        for header_expected in test_new_headers.iter() {
            let init_header_actual =
                get_header(&mut storage, header_expected.hash.as_ref()).unwrap();
            assert_eq!(*header_expected, init_header_actual);

            let actual_height = BTC_HEIGHTS
                .load(&storage, header_expected.hash.as_ref())
                .unwrap();
            assert_eq!(header_expected.height, actual_height);
        }
    }

    // TODO: more tests on different scenarios, e.g., random number of headers and conflicted headers
}
