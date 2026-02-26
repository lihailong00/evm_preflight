use alloy_primitives::{Address, B256, U256, b256};
use preflight_types::{Finding, Severity, SimulationLog, SimulationResult, hexutil};
use serde_json::json;

const APPROVAL_TOPIC0: B256 =
    b256!("8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925");

pub trait RiskRule: Send + Sync {
    fn code(&self) -> &'static str;
    fn evaluate(&self, sim: &SimulationResult) -> Vec<Finding>;
}

pub struct RiskEngine {
    rules: Vec<Box<dyn RiskRule>>,
}

impl RiskEngine {
    pub fn new(rules: Vec<Box<dyn RiskRule>>) -> Self {
        Self { rules }
    }

    pub fn evaluate(&self, sim: &SimulationResult) -> Vec<Finding> {
        self.rules
            .iter()
            .flat_map(|rule| rule.evaluate(sim))
            .collect()
    }
}

impl Default for RiskEngine {
    fn default() -> Self {
        Self::new(vec![
            Box::new(RuleRevertWillFail),
            Box::new(RuleErc20UnlimitedApproval),
        ])
    }
}

pub struct RuleRevertWillFail;

impl RiskRule for RuleRevertWillFail {
    fn code(&self) -> &'static str {
        "RULE_REVERT_WILL_FAIL"
    }

    fn evaluate(&self, sim: &SimulationResult) -> Vec<Finding> {
        if sim.success {
            return Vec::new();
        }

        let details = sim.revert_reason.as_ref().map_or_else(
            || String::from("Transaction is expected to fail under the selected block state."),
            |reason| {
                format!("Transaction is expected to fail under the selected block state: {reason}")
            },
        );

        vec![Finding {
            code: self.code().to_string(),
            severity: Severity::High,
            title: String::from("Transaction will fail"),
            details,
            evidence: json!({
                "success": sim.success,
                "gas_used": sim.gas_used,
                "revert_reason": sim.revert_reason,
            }),
        }]
    }
}

pub struct RuleErc20UnlimitedApproval;

impl RiskRule for RuleErc20UnlimitedApproval {
    fn code(&self) -> &'static str {
        "RULE_ERC20_UNLIMITED_APPROVAL"
    }

    fn evaluate(&self, sim: &SimulationResult) -> Vec<Finding> {
        sim.logs
            .iter()
            .enumerate()
            .filter_map(|(log_index, log)| {
                let approval = parse_approval_log(log)?;
                if !is_unlimited_approval(approval.value) {
                    return None;
                }

                let details = format!(
                    "Token {} emitted Approval(owner={}, spender={}, value={}), which is effectively unlimited.",
                    log.address, approval.owner, approval.spender, approval.value_hex
                );

                Some(Finding {
                    code: self.code().to_string(),
                    severity: Severity::Medium,
                    title: String::from("Unlimited token approval detected"),
                    details,
                    evidence: json!({
                        "log_index": log_index,
                        "token": log.address,
                        "owner": approval.owner,
                        "spender": approval.spender,
                        "value": approval.value_hex,
                    }),
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalEvent {
    pub owner: String,
    pub spender: String,
    pub value: U256,
    pub value_hex: String,
}

pub fn parse_approval_log(log: &SimulationLog) -> Option<ApprovalEvent> {
    if log.topics.len() < 3 {
        return None;
    }

    let topic0 = hexutil::parse_b256(&log.topics[0]).ok()?;
    if topic0 != APPROVAL_TOPIC0 {
        return None;
    }

    let owner = parse_indexed_address(&log.topics[1])?;
    let spender = parse_indexed_address(&log.topics[2])?;
    let value = parse_u256_data_word(&log.data)?;

    Some(ApprovalEvent {
        owner,
        spender,
        value_hex: hexutil::to_lower_hex_u256(value),
        value,
    })
}

fn parse_indexed_address(topic: &str) -> Option<String> {
    let value = hexutil::parse_b256(topic).ok()?;
    let bytes = value.as_slice();
    let address = Address::from_slice(&bytes[12..32]);
    Some(hexutil::to_lower_hex_address(address))
}

fn parse_u256_data_word(data: &str) -> Option<U256> {
    let bytes = hexutil::parse_bytes(data).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    Some(U256::from_be_slice(bytes.as_ref()))
}

fn is_unlimited_approval(value: U256) -> bool {
    let threshold = U256::from(1_u8) << 255;
    value == U256::MAX || value >= threshold
}

#[cfg(test)]
mod tests {
    use super::{
        RiskEngine, RiskRule, RuleErc20UnlimitedApproval, RuleRevertWillFail,
        is_unlimited_approval, parse_approval_log,
    };
    use alloy_primitives::U256;
    use preflight_types::{SimulationLog, SimulationResult, hexutil};

    fn indexed_topic(address: &str) -> String {
        let address = hexutil::parse_address(address).expect("address must parse");
        let mut word = [0_u8; 32];
        word[12..].copy_from_slice(address.as_slice());
        hexutil::to_lower_hex_bytes(&word)
    }

    fn u256_word(value: U256) -> String {
        let word = value.to_be_bytes::<32>();
        hexutil::to_lower_hex_bytes(&word)
    }

    #[test]
    fn parses_approval_log() {
        let log = SimulationLog {
            address: String::from("0x000000000000000000000000000000000000babe"),
            topics: vec![
                String::from("0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925"),
                indexed_topic("0x000000000000000000000000000000000000c0de"),
                indexed_topic("0x000000000000000000000000000000000000beef"),
            ],
            data: u256_word(U256::MAX),
        };

        let parsed = parse_approval_log(&log).expect("must parse approval log");
        assert_eq!(parsed.owner, "0x000000000000000000000000000000000000c0de");
        assert_eq!(parsed.spender, "0x000000000000000000000000000000000000beef");
        assert_eq!(parsed.value, U256::MAX);
    }

    #[test]
    fn checks_unlimited_threshold() {
        assert!(is_unlimited_approval(U256::MAX));
        assert!(is_unlimited_approval(U256::from(1_u8) << 255));
        assert!(!is_unlimited_approval(U256::from(1_000_000_u64)));
    }

    #[test]
    fn emits_revert_rule_finding() {
        let sim = SimulationResult {
            success: false,
            gas_used: 50_000,
            revert_reason: Some(String::from("not allowed")),
            logs: Vec::new(),
            execution_time_ms: 1,
        };
        let findings = RuleRevertWillFail.evaluate(&sim);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "RULE_REVERT_WILL_FAIL");
    }

    #[test]
    fn emits_unlimited_approval_finding() {
        let sim = SimulationResult {
            success: true,
            gas_used: 70_000,
            revert_reason: None,
            logs: vec![SimulationLog {
                address: String::from("0x000000000000000000000000000000000000babe"),
                topics: vec![
                    String::from(
                        "0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925",
                    ),
                    indexed_topic("0x000000000000000000000000000000000000c0de"),
                    indexed_topic("0x000000000000000000000000000000000000beef"),
                ],
                data: u256_word(U256::MAX),
            }],
            execution_time_ms: 1,
        };

        let findings = RuleErc20UnlimitedApproval.evaluate(&sim);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "RULE_ERC20_UNLIMITED_APPROVAL");
    }

    #[test]
    fn risk_engine_evaluates_all_rules() {
        let engine = RiskEngine::default();
        let sim = SimulationResult {
            success: false,
            gas_used: 100_000,
            revert_reason: Some(String::from("execution reverted")),
            logs: Vec::new(),
            execution_time_ms: 2,
        };
        let findings = engine.evaluate(&sim);
        assert!(!findings.is_empty());
    }
}
