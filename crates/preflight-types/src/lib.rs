use alloy_primitives::{Address, B256, Bytes, U256, hex};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulateRequest {
    pub rpc_url: String,
    pub block: BlockInput,
    pub tx: TxInput,
    #[serde(default)]
    pub options: SimulationOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BlockInput {
    Tag(BlockTag),
    Number(u64),
}

impl Default for BlockInput {
    fn default() -> Self {
        Self::Tag(BlockTag::Latest)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockTag {
    Latest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxInput {
    pub from: String,
    pub to: String,
    pub data: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationOptions {
    #[serde(default = "default_true")]
    pub disable_balance_check: bool,
}

fn default_true() -> bool {
    true
}

impl Default for SimulationOptions {
    fn default() -> Self {
        Self {
            disable_balance_check: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub success: bool,
    pub gas_used: u64,
    pub revert_reason: Option<String>,
    pub logs: Vec<SimulationLog>,
    pub execution_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationLog {
    pub address: String,
    pub topics: Vec<String>,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub code: String,
    pub severity: Severity,
    pub title: String,
    pub details: String,
    pub evidence: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulateResponse {
    pub simulation: SimulationResult,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub error: ErrorObject,
}

impl ErrorEnvelope {
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        details: Option<Value>,
    ) -> Self {
        Self {
            error: ErrorObject {
                code: code.into(),
                message: message.into(),
                details,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

pub mod hexutil {
    use super::*;

    pub fn parse_address(value: &str) -> Result<Address, String> {
        let bytes = parse_prefixed_hex(value, "address")?;
        if bytes.len() != 20 {
            return Err(format!(
                "invalid address length: expected 20 bytes, got {}",
                bytes.len()
            ));
        }
        Ok(Address::from_slice(&bytes))
    }

    pub fn parse_b256(value: &str) -> Result<B256, String> {
        let bytes = parse_prefixed_hex(value, "topic/hash")?;
        if bytes.len() != 32 {
            return Err(format!(
                "invalid topic/hash length: expected 32 bytes, got {}",
                bytes.len()
            ));
        }
        Ok(B256::from_slice(&bytes))
    }

    pub fn parse_bytes(value: &str) -> Result<Bytes, String> {
        let bytes = parse_prefixed_hex(value, "bytes")?;
        Ok(Bytes::from(bytes))
    }

    pub fn parse_u256(value: &str) -> Result<U256, String> {
        let bytes = parse_prefixed_hex(value, "uint256")?;
        if bytes.len() > 32 {
            return Err(format!(
                "invalid uint256 length: expected up to 32 bytes, got {}",
                bytes.len()
            ));
        }
        if bytes.is_empty() {
            return Ok(U256::ZERO);
        }
        Ok(U256::from_be_slice(&bytes))
    }

    pub fn to_lower_hex_address(address: Address) -> String {
        format!("0x{}", hex::encode(address.as_slice()))
    }

    pub fn to_lower_hex_b256(value: B256) -> String {
        format!("0x{}", hex::encode(value.as_slice()))
    }

    pub fn to_lower_hex_bytes(data: &[u8]) -> String {
        format!("0x{}", hex::encode(data))
    }

    pub fn to_lower_hex_u256(value: U256) -> String {
        format!("{value:#x}")
    }

    fn parse_prefixed_hex(value: &str, label: &str) -> Result<Vec<u8>, String> {
        let raw = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .ok_or_else(|| format!("{label} must start with 0x"))?;

        let normalized = if raw.len() % 2 == 1 {
            format!("0{raw}")
        } else {
            raw.to_owned()
        };

        hex::decode(normalized).map_err(|err| format!("invalid {label} hex: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::hexutil;
    use alloy_primitives::U256;

    #[test]
    fn u256_roundtrip_hex() {
        let value = hexutil::parse_u256("0xffff").expect("parse must succeed");
        assert_eq!(value, U256::from(65_535_u64));
        assert_eq!(hexutil::to_lower_hex_u256(value), "0xffff");
    }
}
