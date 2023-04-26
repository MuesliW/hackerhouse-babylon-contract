use babylon_bitcoin::BlockHeader;
use babylon_bitcoin::Transaction;
use babylon_proto::babylon::btccheckpoint::v1::TransactionInfo;
use babylon_proto::babylon::checkpointing::v1::{
    CURRENT_VERSION, FIRST_PART_LEN, HEADER_LEN, MERKLE_PROOF_ELEM_SIZE, SECOND_PART_LEN, TAG_LEN,
};

/// parse_tx_info checks whether the given tx_info is correct against the given btc_header, i.e.,
/// - the BTC header hash in tx_info is same as the btc_header's hash
/// - the Merkle proof in tx_info proves that the tx in tx_info is committed to btc_header
/// If the checks passed, return the decoded tx
pub fn parse_tx_info(
    tx_info: &TransactionInfo,
    btc_header: &BlockHeader,
) -> Result<Transaction, String> {
    // get Merkle root
    let root = btc_header.merkle_root.as_hash();

    // get proof
    let proof_bytes = &tx_info.proof;
    let proof_chunks = proof_bytes.chunks_exact(MERKLE_PROOF_ELEM_SIZE);
    if !proof_chunks.remainder().is_empty() {
        return Err("proof has a remainder".to_string());
    }
    let proof: Vec<&[u8]> = proof_chunks.collect();

    // get tx key
    let tx_key_res = &tx_info.key;
    if tx_key_res.is_none() {
        return Err("empty tx key".to_string());
    }
    let tx_key = tx_key_res.as_ref().unwrap();

    // get header hash in tx key and tx idx
    let header_hash = tx_key.hash.to_vec();
    let tx_idx = tx_key.index as usize;

    // compare header hash in tx key and the given header's hash
    if !btc_header.block_hash().to_vec().eq(&header_hash) {
        return Err("BTC header does not match".to_string());
    }

    // deserialise btc tx
    let btc_tx_res: Result<Transaction, babylon_bitcoin::Error> =
        babylon_bitcoin::deserialize(&tx_info.transaction);
    if btc_tx_res.is_err() {
        return Err("failed to decode BTC tx".to_string());
    }
    let btc_tx = btc_tx_res.unwrap();

    // verify Merkle proof
    if !babylon_bitcoin::merkle::verify_merkle_proof(&btc_tx, &proof, tx_idx, &root) {
        return Err("failed to verify Bitcoin Merkle proof".to_string());
    }

    Ok(btc_tx)
}

/// extract_checkpoint_data extracts the checkpoint data of the given tx
pub fn extract_checkpoint_data(
    btc_tx: &Transaction,
    tag: &[u8],
    idx: usize,
) -> Result<Vec<u8>, String> {
    // get OP_RETURN data
    let op_return_data = babylon_bitcoin::op_return::extract_op_return_data(&btc_tx)?;

    // verify OP_RETURN length
    if idx == 0 && op_return_data.len() != FIRST_PART_LEN {
        return Err(format!(
            "invalid length. First part should have {} bytes",
            FIRST_PART_LEN
        ));
    }
    if idx == 1 && op_return_data.len() != SECOND_PART_LEN {
        return Err(format!(
            "invalid length. Second part should have {} bytes",
            SECOND_PART_LEN
        ));
    }
    // verify tag
    if tag.ne(&op_return_data[0..TAG_LEN]) {
        return Err(format!(
            "data does not have expected tag, expected tag: {:?}, got tag: {:?}",
            tag,
            &op_return_data[0..TAG_LEN]
        ));
    }
    // verify version
    let ver_half = op_return_data[TAG_LEN];
    let version = ver_half & 0xf;
    if version > CURRENT_VERSION {
        return Err(format!("header have invalid version"));
    }
    // verify idx
    let part = ver_half >> 4;
    if idx != part as usize {
        return Err(format!("header have invalid part number"));
    }

    let checkpoint_data = op_return_data[HEADER_LEN..op_return_data.len()].to_vec();
    return Ok(checkpoint_data);
}
