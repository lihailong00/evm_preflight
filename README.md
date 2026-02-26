# evm-preflight

Self-hosted EVM transaction simulator + risk engine in Rust.

## What It Does

- Simulates a single EVM transaction against a chosen RPC endpoint and block state.
- Returns simulation output:
  - `success`
  - `gas_used`
  - `revert_reason` (tries to decode standard `Error(string)`)
  - raw `logs`
  - `execution_time_ms`
- Runs a pluggable risk engine (MVP includes 2 rules):
  - `RULE_REVERT_WILL_FAIL` (HIGH)
  - `RULE_ERC20_UNLIMITED_APPROVAL` (MEDIUM)

## Disclaimer

This tool performs pre-execution against a specific chain state (selected block / latest snapshot).
It does **not** model mempool competition, frontrunning, sandwich attacks, bundle ordering, or state changes after the chosen block.
Simulation results are best-effort previews, not execution guarantees.

## MVP Scope

Implemented:
- HTTP API (`axum`)
  - `GET /healthz`
  - `POST /v1/simulate`
- CLI
  - `evm-preflight simulate --rpc ... --block ... --from ... --to ... --data ... --value ...`
- Core simulation stack:
  - `alloy-provider` + `revm::database::AlloyDB` + `WrapDatabaseAsync` + `CacheDB`
- Risk engine with the two required rules
- Unit tests + optional live integration test

Not implemented (non-goals):
- mempool simulation
- sandwich/path/MEV route search
- `debug_traceCall` style deep tracing
- token USD pricing

## Quick Start

Requirements:
- Rust stable (`cargo`, `rustc`)

Install deps and build:

```bash
cargo build
```

Run API server:

```bash
cargo run -p preflight-api
```

Default bind: `0.0.0.0:3000`

Run CLI:

```bash
cargo run -p preflight-cli -- simulate \
  --rpc https://YOUR_RPC \
  --block latest \
  --from 0x0000000000000000000000000000000000000001 \
  --to 0x0000000000000000000000000000000000000002 \
  --data 0x \
  --value 0x0
```

## HTTP API

### `GET /healthz`

Response:

```text
ok
```

### `POST /v1/simulate`

Request:

```json
{
  "rpc_url": "https://your-rpc.example",
  "block": "latest",
  "tx": {
    "from": "0x0000000000000000000000000000000000000001",
    "to": "0x0000000000000000000000000000000000000002",
    "data": "0x",
    "value": "0x0"
  },
  "options": {
    "disable_balance_check": true
  }
}
```

Example `curl`:

```bash
curl -sS http://127.0.0.1:3000/v1/simulate \
  -H 'content-type: application/json' \
  -d '{
    "rpc_url":"https://your-rpc.example",
    "block":"latest",
    "tx":{
      "from":"0x0000000000000000000000000000000000000001",
      "to":"0x0000000000000000000000000000000000000002",
      "data":"0x",
      "value":"0x0"
    },
    "options":{"disable_balance_check":true}
  }'
```

Success response shape:

```json
{
  "simulation": {
    "success": true,
    "gas_used": 21000,
    "revert_reason": null,
    "logs": [],
    "execution_time_ms": 3
  },
  "findings": []
}
```

Error response shape:

```json
{
  "error": {
    "code": "BAD_REQUEST|RPC_ERROR|SIMULATION_ERROR",
    "message": "..."
  }
}
```

## Project Structure

```text
crates/
  preflight-types/   # serde DTOs and shared schema
  preflight-sim/     # revm + alloy simulation core
  preflight-risk/    # risk rules and engine
  preflight-api/     # axum HTTP service
  preflight-cli/     # command line interface
```

## Developer Commands

Format:

```bash
cargo fmt --check
```

Lint:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Test:

```bash
cargo test --workspace
```

Optional live RPC integration test:

```bash
ETH_RPC_URL=https://your-rpc.example cargo test -p preflight-sim --test live_rpc -- --nocapture
```

If `ETH_RPC_URL` is not set, the live test is skipped automatically.

## FAQ

### Why is `disable_balance_check` enabled by default?

The MVP prioritizes deterministic preflight execution. Real accounts used for simulation may not hold enough native token to satisfy transaction fee checks in `revm`, especially for dry-run callers.  
To reduce false negatives:
- balance check is disabled by default, and
- the sender is locally topped up in the simulation cache (state is not written on-chain).

This behavior only affects in-memory simulation state.
