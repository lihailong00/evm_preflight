# evm-preflight

使用 Rust 构建的自托管 EVM 交易仿真与风控引擎。

## 项目功能

- 在指定 RPC 节点与区块状态下，仿真单笔 EVM 交易。
- 返回仿真结果：
  - `success`
  - `gas_used`
  - `revert_reason`（尽量解析标准 `Error(string)`）
  - 原始 `logs`
  - `execution_time_ms`
- 执行可扩展风控规则（MVP 内置 2 条）：
  - `RULE_REVERT_WILL_FAIL` (HIGH)
  - `RULE_ERC20_UNLIMITED_APPROVAL` (MEDIUM)

## 免责声明

本工具在指定链上状态（指定区块 / 最新快照）下进行预执行仿真。
它**不会**模拟 mempool 竞争、抢跑、夹子攻击、打包顺序或所选区块之后的状态变化。
仿真结果仅用于预演参考，不保证真实链上执行结果。

## MVP 范围

已实现：
- HTTP API（`axum`）
  - `GET /healthz`
  - `POST /v1/simulate`
- CLI
  - `evm-preflight simulate --rpc ... --block ... --from ... --to ... --data ... --value ...`
- 核心仿真栈：
  - `alloy-provider` + `revm::database::AlloyDB` + `WrapDatabaseAsync` + `CacheDB`
- 两条必需风控规则
- 单元测试 + 可选在线集成测试

未实现（非目标）：
- mempool 仿真
- 夹子/路径/MEV 路由搜索
- `debug_traceCall` 风格的深度 tracing
- Token 美元定价

## 快速开始

环境要求：
- Rust 稳定版（`cargo`, `rustc`）

安装依赖并构建：

```bash
cargo build
```

启动 API 服务：

```bash
cargo run -p preflight-api
```

默认监听：`0.0.0.0:3000`

运行 CLI：

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

返回：

```text
ok
```

### `POST /v1/simulate`

请求：

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

`curl` 示例：

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

成功响应结构：

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

错误响应结构：

```json
{
  "error": {
    "code": "BAD_REQUEST|RPC_ERROR|SIMULATION_ERROR",
    "message": "..."
  }
}
```

## 项目结构

```text
crates/
  preflight-types/   # serde 数据结构与共享 schema
  preflight-sim/     # revm + alloy 仿真核心
  preflight-risk/    # 风控规则与引擎
  preflight-api/     # axum HTTP 服务
  preflight-cli/     # 命令行工具
```

## 开发命令

格式检查：

```bash
cargo fmt --check
```

静态检查：

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

测试：

```bash
cargo test --workspace
```

可选在线 RPC 集成测试：

```bash
ETH_RPC_URL=https://your-rpc.example cargo test -p preflight-sim --test live_rpc -- --nocapture
```

如果未设置 `ETH_RPC_URL`，在线测试会自动跳过。

## FAQ

### 为什么默认开启 `disable_balance_check`？

MVP 优先保证预检仿真可稳定执行。用于仿真的真实账户可能没有足够原生代币通过 `revm` 的费用校验，尤其是仅预演调用（dry-run）场景。  
为降低误报失败：
- 默认关闭余额校验；
- 在本地仿真缓存中临时补足 sender 余额（不会写入链上状态）。

该行为仅影响内存中的仿真状态。
