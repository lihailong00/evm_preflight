use preflight_sim::simulate;
use preflight_types::{BlockInput, BlockTag, SimulateRequest, SimulationOptions, TxInput};

#[tokio::test(flavor = "multi_thread")]
async fn live_rpc_smoke_test() {
    let Some(rpc_url) = std::env::var("ETH_RPC_URL").ok() else {
        eprintln!("Skipping live RPC smoke test because ETH_RPC_URL is not set.");
        return;
    };

    let request = SimulateRequest {
        rpc_url,
        block: BlockInput::Tag(BlockTag::Latest),
        tx: TxInput {
            from: String::from("0x0000000000000000000000000000000000000001"),
            to: String::from("0x0000000000000000000000000000000000000002"),
            data: String::from("0x"),
            value: String::from("0x0"),
        },
        options: SimulationOptions {
            disable_balance_check: true,
        },
    };

    let result = simulate(&request)
        .await
        .expect("live simulation should succeed");
    assert!(result.success, "expected success from empty call");
    assert_eq!(result.revert_reason, None);
}
