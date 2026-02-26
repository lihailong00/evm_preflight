use std::time::Instant;

use alloy_consensus::BlockHeader;
use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use anyhow::Context as _;
use preflight_types::{
    BlockInput, BlockTag, SimulateRequest, SimulationLog, SimulationResult, hexutil,
};
use revm::{
    Database, ExecuteEvm, MainBuilder, MainContext,
    context::{BlockEnv, Context, TxEnv},
    context_interface::result::{EVMError, ExecutionResult, InvalidTransaction},
    database::{AlloyDB, BlockId, CacheDB},
    database_interface::WrapDatabaseAsync,
    primitives::{TxKind, U256},
};
use thiserror::Error;

const DEFAULT_TX_GAS_LIMIT: u64 = 30_000_000;
const ERROR_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];

#[derive(Debug, Error)]
pub enum SimError {
    #[error("{0}")]
    BadInput(String),
    #[error("{0}")]
    Rpc(String),
    #[error("{0}")]
    Simulation(String),
}

pub type Result<T> = std::result::Result<T, SimError>;

pub async fn simulate(request: &SimulateRequest) -> Result<SimulationResult> {
    let from = hexutil::parse_address(&request.tx.from).map_err(SimError::BadInput)?;
    let to = hexutil::parse_address(&request.tx.to).map_err(SimError::BadInput)?;
    let data = hexutil::parse_bytes(&request.tx.data).map_err(SimError::BadInput)?;
    let value = hexutil::parse_u256(&request.tx.value).map_err(SimError::BadInput)?;
    let block_id = block_id_from_input(&request.block);

    let provider = connect_provider(&request.rpc_url).await?;
    let block_env = fetch_block_env(&provider, block_id).await?;

    let tx_env = TxEnv::builder()
        .caller(from)
        .kind(TxKind::Call(to))
        .data(data)
        .value(value)
        .gas_limit(DEFAULT_TX_GAS_LIMIT)
        .gas_price(0)
        .nonce(0)
        .chain_id(None)
        .build_fill();

    let alloy_db = AlloyDB::new(provider, block_id);
    let wrapped_db = wrap_async_db(alloy_db)?;
    let mut cache_db = CacheDB::new(wrapped_db);

    if request.options.disable_balance_check {
        let mut account = cache_db
            .basic(from)
            .map_err(|err| SimError::Rpc(format!("failed to load sender account: {err}")))?
            .unwrap_or_default();
        account.balance = U256::MAX;
        cache_db.insert_account_info(from, account);
    }

    let context = Context::mainnet()
        .with_block(block_env)
        .with_db(cache_db)
        .modify_cfg_chained(|cfg| {
            cfg.disable_nonce_check = true;
            cfg.tx_chain_id_check = false;
            cfg.disable_balance_check = request.options.disable_balance_check;
            cfg.disable_block_gas_limit = true;
            cfg.disable_base_fee = true;
            cfg.disable_fee_charge = true;
        });

    let mut evm = context.build_mainnet();
    let started_at = Instant::now();
    let outcome = evm.transact(tx_env).map_err(map_evm_error)?;
    let execution_time_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);

    let result = outcome.result;
    let gas_used = result.gas_used();
    let (success, revert_reason, logs) = map_execution_result(result);

    Ok(SimulationResult {
        success,
        gas_used,
        revert_reason,
        logs,
        execution_time_ms,
    })
}

fn map_execution_result(result: ExecutionResult) -> (bool, Option<String>, Vec<SimulationLog>) {
    match result {
        ExecutionResult::Success { logs, .. } => {
            let raw_logs = logs.iter().map(map_log).collect();
            (true, None, raw_logs)
        }
        ExecutionResult::Revert { output, .. } => {
            let reason = decode_revert_reason(output.as_ref())
                .or_else(|| Some(String::from("execution reverted")));
            (false, reason, Vec::new())
        }
        ExecutionResult::Halt { reason, .. } => (
            false,
            Some(format!("execution halted: {reason}")),
            Vec::new(),
        ),
    }
}

fn map_log(log: &revm::primitives::Log) -> SimulationLog {
    let topics = log
        .data
        .topics()
        .iter()
        .map(|topic| hexutil::to_lower_hex_b256(*topic))
        .collect();
    let data = hexutil::to_lower_hex_bytes(log.data.data.as_ref());

    SimulationLog {
        address: hexutil::to_lower_hex_address(log.address),
        topics,
        data,
    }
}

fn map_evm_error<DBError>(err: EVMError<DBError, InvalidTransaction>) -> SimError
where
    DBError: std::fmt::Display,
{
    match err {
        EVMError::Database(db_err) => SimError::Rpc(format!("rpc database error: {db_err}")),
        EVMError::Transaction(tx_err) => {
            SimError::Simulation(format!("transaction validation failed: {tx_err}"))
        }
        EVMError::Header(header_err) => {
            SimError::Simulation(format!("invalid header while simulating: {header_err}"))
        }
        EVMError::Custom(msg) => SimError::Simulation(msg),
    }
}

fn block_id_from_input(input: &BlockInput) -> BlockId {
    match input {
        BlockInput::Tag(BlockTag::Latest) => BlockId::latest(),
        BlockInput::Number(number) => BlockId::number(*number),
    }
}

async fn connect_provider(rpc_url: &str) -> Result<DynProvider> {
    ProviderBuilder::default()
        .connect(rpc_url)
        .await
        .map(|provider| provider.erased())
        .map_err(|err| SimError::Rpc(format!("failed to connect rpc provider: {err}")))
}

async fn fetch_block_env(provider: &DynProvider, block_id: BlockId) -> Result<BlockEnv> {
    let header = provider
        .get_header(block_id)
        .await
        .map_err(|err| SimError::Rpc(format!("failed to fetch block header: {err}")))?
        .ok_or_else(|| SimError::Rpc(format!("requested block {block_id:?} not found")))?;

    let mut block = BlockEnv {
        number: U256::from(header.number()),
        beneficiary: header.beneficiary(),
        timestamp: U256::from(header.timestamp()),
        gas_limit: header.gas_limit(),
        basefee: header.base_fee_per_gas().unwrap_or(0),
        difficulty: header.difficulty(),
        prevrandao: header.mix_hash(),
        blob_excess_gas_and_price: None,
    };

    if let Some(excess_blob_gas) = header.excess_blob_gas() {
        block.set_blob_excess_gas_and_price(
            excess_blob_gas,
            revm::primitives::eip4844::BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
        );
    }

    Ok(block)
}

fn wrap_async_db<T>(db: T) -> Result<WrapDatabaseAsync<T>> {
    if let Ok(handle) = tokio::runtime::Handle::try_current()
        && !matches!(
            handle.runtime_flavor(),
            tokio::runtime::RuntimeFlavor::CurrentThread
        )
    {
        return Ok(WrapDatabaseAsync::with_handle(db, handle));
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .context("failed to create fallback tokio runtime for WrapDatabaseAsync")
        .map_err(|err| SimError::Simulation(err.to_string()))?;

    Ok(WrapDatabaseAsync::with_runtime(db, runtime))
}

pub fn decode_revert_reason(data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return None;
    }
    if data.len() < 4 || data[..4] != ERROR_SELECTOR {
        return Some(hexutil::to_lower_hex_bytes(data));
    }
    if data.len() < 68 {
        return Some(hexutil::to_lower_hex_bytes(data));
    }

    let offset = decode_abi_word_to_usize(&data[4..36])?;
    let offset_start = 4usize.checked_add(offset)?;
    let len_end = offset_start.checked_add(32)?;
    if len_end > data.len() {
        return Some(hexutil::to_lower_hex_bytes(data));
    }

    let string_len = decode_abi_word_to_usize(&data[offset_start..len_end])?;
    let string_start = len_end;
    let string_end = string_start.checked_add(string_len)?;
    if string_end > data.len() {
        return Some(hexutil::to_lower_hex_bytes(data));
    }

    match std::str::from_utf8(&data[string_start..string_end]) {
        Ok(reason) => Some(reason.to_owned()),
        Err(_) => Some(hexutil::to_lower_hex_bytes(data)),
    }
}

fn decode_abi_word_to_usize(word: &[u8]) -> Option<usize> {
    if word.len() != 32 {
        return None;
    }
    if word[..24].iter().any(|byte| *byte != 0) {
        return None;
    }

    let mut result: usize = 0;
    for byte in &word[24..] {
        result = result.checked_mul(256)?;
        result = result.checked_add(usize::from(*byte))?;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::decode_revert_reason;

    fn encode_revert_reason(message: &str) -> Vec<u8> {
        let payload = message.as_bytes();
        let mut out = Vec::new();
        out.extend_from_slice(&[0x08, 0xc3, 0x79, 0xa0]);

        let mut offset = [0_u8; 32];
        offset[31] = 32;
        out.extend_from_slice(&offset);

        let mut len = [0_u8; 32];
        len[31] = u8::try_from(payload.len()).expect("payload must fit into u8");
        out.extend_from_slice(&len);

        out.extend_from_slice(payload);
        while out.len() % 32 != 0 {
            out.push(0);
        }

        out
    }

    #[test]
    fn parses_standard_error_string() {
        let bytes = encode_revert_reason("not allowed");
        let parsed = decode_revert_reason(&bytes);
        assert_eq!(parsed.as_deref(), Some("not allowed"));
    }

    #[test]
    fn falls_back_to_hex_for_unknown_payload() {
        let bytes = vec![0xde, 0xad, 0xbe, 0xef];
        let parsed = decode_revert_reason(&bytes);
        assert_eq!(parsed.as_deref(), Some("0xdeadbeef"));
    }

    #[test]
    fn returns_none_for_empty_payload() {
        assert!(decode_revert_reason(&[]).is_none());
    }
}
