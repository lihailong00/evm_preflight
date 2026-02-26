# EVM Preflight 入门指南（面向 Rust 开发者）

本文面向“有 Rust 工程经验，但刚接触 Web3 / EVM”的开发者，帮助你快速理解：

- EVM 是什么、交易是如何执行的
- 仿真（simulation）到底在做什么
- 本项目 `evm-preflight` 的架构与关键代码路径
- 如何在本地运行、调试、扩展风控规则

---

## 1. 你已有的能力，如何迁移到 Web3

如果你已经熟悉 Rust，你在这个项目里已经具备了大量优势：

- **类型系统与边界意识**：在链上场景尤其重要（地址、金额、编码）。
- **错误处理经验**：RPC 失败、仿真失败、规则误报都需要明确错误边界。
- **性能与并发理解**：RPC IO + 本地执行器 + API/CLI 集成，本质是系统工程问题。
- **工程化习惯**：`fmt/clippy/test` 在链上服务同样是硬要求。

你需要补齐的，主要是 EVM 领域知识，而不是 Rust 本身。

---

## 2. EVM 核心概念（最小必要集）

### 2.1 账户模型：EOA vs Contract

- **EOA（外部账户）**：由私钥控制，常见“钱包地址”。
- **Contract（合约账户）**：由字节码控制，执行逻辑由 EVM 解释。

本项目中，你提供的 `from` 常是 EOA，`to` 通常是合约地址。

### 2.2 交易输入四元组：`from / to / data / value`

- `from`：调用发起方
- `to`：目标地址（MVP 中是 Call，不做 Create）
- `data`：ABI 编码后的函数选择器 + 参数
- `value`：转账金额（wei）

### 2.3 Gas 与失败语义

- **Revert**：合约主动回滚，常带 `Error(string)`。
- **Halt**：执行异常中止（如 out-of-gas）。
- 两者都属于“交易失败”，但语义不同。

本项目把这两种都映射为 `success = false`，并尽力给出 `revert_reason`。

### 2.4 Logs / Events

EVM 日志由 `address + topics + data` 组成：

- `topic0` 通常是事件签名哈希（如 `Approval(address,address,uint256)`）
- `topics[1..]` 常承载 indexed 参数
- `data` 承载非 indexed 参数

本项目的无限授权规则就是基于日志解析。

---

## 3. “交易仿真”到底在做什么

你可以把它理解为：

1. 选定一个链状态快照（某个区块）
2. 把你给定的交易输入塞进本地 EVM
3. 执行后提取结果（是否成功、gas、日志、回滚信息）

**关键点**：仿真结果依赖“所选区块状态”。  
它不等价于“真实上链后必然结果”，因为真实链上还受 mempool 竞争、先后顺序、状态变化影响。

---

## 4. 本项目架构总览

目录（workspace）：

```text
crates/
  preflight-types/
  preflight-sim/
  preflight-risk/
  preflight-api/
  preflight-cli/
```

模块职责：

- `preflight-types`：纯数据结构（请求/响应/日志/finding/错误信封）
- `preflight-sim`：仿真执行核心（Alloy Provider + revm）
- `preflight-risk`：规则引擎与规则实现（不依赖 revm）
- `preflight-api`：HTTP 服务拼装层（axum）
- `preflight-cli`：命令行入口

这种分层非常重要：**仿真核心与对外协议解耦**，后续才能平滑扩展。

---

## 5. 仿真执行链路（重点）

核心文件：`crates/preflight-sim/src/lib.rs`

### 5.1 Provider 与链状态

- 使用 `alloy-provider` 连接 RPC
- 根据请求构造 `BlockId`（`latest` 或 number）
- 读取区块头并映射到 `revm::context::BlockEnv`

### 5.2 数据库：`AlloyDB + WrapDatabaseAsync + CacheDB`

- `AlloyDB`：按需从 RPC 拉账户、存储、代码
- `WrapDatabaseAsync`：给 revm 提供同步访问外观
- `CacheDB`：缓存状态读取，减少重复 RPC

这套组合实现了“远程状态 + 本地执行”的最小闭环。

### 5.3 交易环境构建

构建 `TxEnv` 时设置：

- caller / kind(Call) / data / value
- gas limit（默认 30,000,000）
- gas price 设为 0

### 5.4 为何默认关闭余额检查等开关

MVP 目标是“可稳定预演”，不是“完全复刻主网费用机制”。  
因此默认：

- `disable_balance_check = true`
- `disable_base_fee = true`
- `disable_fee_charge = true`

并且对 sender 在 `CacheDB` 中本地注入大余额作为兜底，避免 `LackOfFundForMaxFee` 类失败影响可用性。

> 这只影响仿真内存状态，不会写回链上。

### 5.5 结果提取

从 `ExecutionResult` 提取：

- `success`
- `gas_used`
- `logs`（成功时）
- `revert_reason`（Revert/Halt 时）

其中 `revert_reason` 支持标准 `Error(string)` 选择器 `0x08c379a0` 解析，失败时回退到 hex 字符串。

---

## 6. 风控引擎设计与实现

核心文件：`crates/preflight-risk/src/lib.rs`

### 6.1 规则接口

通过 `RiskRule` trait 定义规则：

- `code() -> &'static str`
- `evaluate(&SimulationResult) -> Vec<Finding>`

好处：

- 新规则可插拔
- 与执行器解耦（只依赖仿真结果）

### 6.2 内置规则 1：`RULE_REVERT_WILL_FAIL`

逻辑很直接：

- `sim.success == false` 时输出 HIGH finding
- details 包含可读失败说明和回滚原因（如果有）

### 6.3 内置规则 2：`RULE_ERC20_UNLIMITED_APPROVAL`

流程：

1. 扫描 `sim.logs`
2. 匹配 `topic0 == Approval` 签名
3. 解析 `owner / spender / value`
4. 判断 `value == U256::MAX` 或 `value >= 2^255`
5. 命中时输出 MEDIUM finding，并附 `log_index` 等 evidence

这个规则是“日志后置分析”范式，后续可扩展更多规则（如 Transfer 异常模式）。

---

## 7. API 层（HTTP）

核心文件：`crates/preflight-api/src/main.rs`

路由：

- `GET /healthz` -> `"ok"`
- `POST /v1/simulate` -> `simulation + findings`

统一错误结构：

```json
{
  "error": {
    "code": "BAD_REQUEST|RPC_ERROR|SIMULATION_ERROR",
    "message": "..."
  }
}
```

错误映射策略：

- 输入解析失败 -> `BAD_REQUEST`
- 上游 RPC/状态拉取失败 -> `RPC_ERROR`
- 执行器内部失败 -> `SIMULATION_ERROR`

---

## 8. CLI 层

核心文件：`crates/preflight-cli/src/main.rs`

命令：

```bash
evm-preflight simulate --rpc <URL> --block <latest|number> --from ... --to ... --data ... --value ...
```

行为：

- 直接调用仿真核心 + 风控引擎
- 输出 prettified JSON
- 出错时输出统一错误 JSON 并返回非 0 退出码

---

## 9. 你可以直接跑起来（最短路径）

### 9.1 启动 API

```bash
cargo run -p preflight-api
```

### 9.2 发起一次仿真请求

```bash
curl -sS http://127.0.0.1:3000/v1/simulate ^
  -H "content-type: application/json" ^
  -d "{\"rpc_url\":\"https://your-rpc.example\",\"block\":\"latest\",\"tx\":{\"from\":\"0x0000000000000000000000000000000000000001\",\"to\":\"0x0000000000000000000000000000000000000002\",\"data\":\"0x\",\"value\":\"0x0\"},\"options\":{\"disable_balance_check\":true}}"
```

### 9.3 CLI 方式

```bash
cargo run -p preflight-cli -- simulate --rpc https://your-rpc.example --block latest --from 0x0000000000000000000000000000000000000001 --to 0x0000000000000000000000000000000000000002 --data 0x --value 0x0
```

---

## 10. 测试与质量门禁

本项目已经配置并通过：

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

测试覆盖重点：

- revert reason 解析
- Approval 日志解析
- 风控规则行为
- 可选在线 RPC smoke test（`ETH_RPC_URL` 环境变量存在时执行）

---

## 11. 常见问题（给 Rust 工程师）

### Q1：为什么不用 `eth_call` 直接替代？

`eth_call` 可以做调用预演，但本项目目标是“可控本地执行 + 可扩展风控后处理”，并希望统一 API/CLI 输出结构，便于自托管集成。

### Q2：为什么 risk crate 不直接依赖 revm？

为了保持规则层稳定与可移植。  
规则应该关注“结果语义”，而不是执行器细节类型。

### Q3：日志为什么只在 success 时有？

按当前 `ExecutionResult` 语义，成功结果包含 logs；失败时会回滚并不返回同样的 logs 结果。  
若你要做更深层失败路径分析，需要引入 trace 能力（当前 MVP 明确不做）。

### Q4：这个结果能否当成“最终交易成功保证”？

不能。它是指定状态快照下的预演，不覆盖真实链上竞争环境。

---

## 12. 下一步扩展建议（工程优先级）

对入门者最友好的扩展顺序：

1. **新增风险规则**：先写纯 `SimulationResult -> Finding` 规则，最快出价值。
2. **增强错误可观测性**：增加 request-id、结构化 tracing 字段。
3. **支持更多 block tag**：如 `safe/finalized`。
4. **引入可选 trace**：仅在高级模式开启，避免影响默认性能与复杂度。
5. **对接持久化**：将仿真结果与 findings 写入数据库供审计。

---

## 13. 结语

对 Rust 开发者来说，Web3 的难点不是语言，而是执行语义与状态模型。  
你可以把 `evm-preflight` 当成一个“把链上语义系统化工程化”的起点：先跑通，先可观测，再逐步提高规则精度与覆盖范围。

