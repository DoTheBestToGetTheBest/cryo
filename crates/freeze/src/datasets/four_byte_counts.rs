use crate::*;
use polars::prelude::*;
use std::collections::BTreeMap;

/// columns for transactions
#[cryo_to_df::to_df(Datatype::FourByteCounts)]
#[derive(Default)]
pub struct FourByteCounts {
    pub(crate) n_rows: u64,
    pub(crate) block_number: Vec<Option<u32>>,
    pub(crate) transaction_index: Vec<Option<u32>>,
    pub(crate) transaction_hash: Vec<Option<Vec<u8>>>,
    pub(crate) signature: Vec<Vec<u8>>,
    pub(crate) size: Vec<u64>,
    pub(crate) count: Vec<u64>,
    pub(crate) chain_id: Vec<u64>,
}

#[async_trait::async_trait]
impl Dataset for FourByteCounts {
    fn aliases() -> Vec<&'static str> {
        vec!["4byte_counts"]
    }
}

type BlockTxsTraces = (Option<u32>, Vec<Option<Vec<u8>>>, Vec<BTreeMap<String, u64>>);

#[async_trait::async_trait]
impl CollectByBlock for FourByteCounts {
    type Response = BlockTxsTraces;

    async fn extract(request: Params, source: Arc<Source>, query: Arc<Query>) -> R<Self::Response> {
        let schema =
            query.schemas.get(&Datatype::FourByteCounts).ok_or(err("schema not provided"))?;
        let include_txs = schema.has_column("transaction_hash");
        source
            .fetcher
            .geth_debug_trace_block_4byte_traces(request.block_number()? as u32, include_txs)
            .await
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        process_storage_reads(&response, columns, &query.schemas)
    }
}

#[async_trait::async_trait]
impl CollectByTransaction for FourByteCounts {
    type Response = BlockTxsTraces;

    async fn extract(request: Params, source: Arc<Source>, query: Arc<Query>) -> R<Self::Response> {
        let schema =
            query.schemas.get(&Datatype::FourByteCounts).ok_or(err("schema not provided"))?;
        let include_block_number = schema.has_column("block_number");
        let tx = request.transaction_hash()?;
        source.fetcher.geth_debug_trace_transaction_4byte_traces(tx, include_block_number).await
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        process_storage_reads(&response, columns, &query.schemas)
    }
}

pub(crate) fn process_storage_reads(
    response: &BlockTxsTraces,
    columns: &mut FourByteCounts,
    schemas: &Schemas,
) -> R<()> {
    let schema = schemas.get(&Datatype::FourByteCounts).ok_or(err("schema not provided"))?;
    let (block_number, txs, traces) = response;
    for (index, (trace, tx)) in traces.iter().zip(txs).enumerate() {
        for (signature_size, count) in trace.iter() {
            let (signature, size) = parse_signature_size(signature_size)?;
            columns.n_rows += 1;
            store!(schema, columns, block_number, *block_number);
            store!(schema, columns, transaction_index, Some(index as u32));
            store!(schema, columns, transaction_hash, tx.clone());
            store!(schema, columns, signature, signature.clone());
            store!(schema, columns, size, size);
            store!(schema, columns, count, *count);
        }
    }
    Ok(())
}

fn parse_signature_size(signature_size: &str) -> Result<(Vec<u8>, u64), CollectError> {
    // Check if the input is a full function signature
    if signature_size.contains('(') {
        let selector = function_signature_to_selector(signature_size);
        return Ok((selector.to_vec(), 0)) // Placeholder for size, adjust as needed
    }

    // Parse the hexadecimal part and the size for a 4byte-size pair
    let parts: Vec<&str> = signature_size.splitn(2, '-').collect();
    if parts.len() != 2 {
        return Err(CollectError::CollectError("could not parse 4byte-size pair".to_string()))
    }

    // Parse the hexadecimal part
    let hex_part = parts[0].trim_start_matches("0x");
    let bytes = (0..hex_part.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex_part[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .map_err(|_| CollectError::CollectError("could not parse signature bytes".to_string()))?;

    // Parse the number as u64
    let number = parts[1]
        .parse::<u64>()
        .map_err(|_| CollectError::CollectError("could not parse call data size".to_string()))?;

    Ok((bytes, number))
}
fn function_signature_to_selector(signature: &str) -> [u8; 4] {
    let hash = ethers_core::utils::keccak256(signature);
    [hash[0], hash[1], hash[2], hash[3]]
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_full_function_signature() {
        let signature = "transfer(address,uint256)";
        let result = parse_signature_size(signature);

        assert!(result.is_ok(), "Parsing full function signature should succeed");
        let (bytes, size) = result.unwrap();
        assert_eq!(bytes.len(), 4, "Should return a 4-byte array for the selector");
        assert_eq!(size, 0, "Placeholder size should be 0 for full function signatures");
    }

    #[test]
    fn test_parse_valid_4byte_size_pair() {
        let signature_size = "a9059cbb-64";
        let result = parse_signature_size(signature_size);

        assert!(result.is_ok(), "Parsing valid 4byte-size pair should succeed");
        let (bytes, size) = result.unwrap();
        assert_eq!(bytes.len(), 4, "Should return a 4-byte array for the selector");
        assert_eq!(size, 64, "Size should be correctly parsed from the input");
    }

    #[test]
    fn test_parse_invalid_input() {
        let invalid_input = "invalid-input";
        let result = parse_signature_size(invalid_input);

        assert!(result.is_err(), "Parsing invalid input should fail");
    }
}
