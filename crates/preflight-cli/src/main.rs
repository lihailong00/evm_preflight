use anyhow::{Context as _, anyhow};
use clap::{Args, Parser, Subcommand};
use preflight_risk::RiskEngine;
use preflight_sim::{SimError, simulate};
use preflight_types::{
    BlockInput, BlockTag, ErrorEnvelope, SimulateRequest, SimulateResponse, SimulationOptions,
    TxInput,
};

#[derive(Debug, Parser)]
#[command(
    name = "evm-preflight",
    about = "Self-hosted EVM transaction simulation CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Simulate(SimulateArgs),
}

#[derive(Debug, Args)]
struct SimulateArgs {
    #[arg(long)]
    rpc: String,
    #[arg(long, default_value = "latest")]
    block: String,
    #[arg(long)]
    from: String,
    #[arg(long)]
    to: String,
    #[arg(long, default_value = "0x")]
    data: String,
    #[arg(long, default_value = "0x0")]
    value: String,
    #[arg(long, default_value_t = true)]
    disable_balance_check: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Simulate(args) => run_simulate(args).await,
    }
}

async fn run_simulate(args: SimulateArgs) -> anyhow::Result<()> {
    let request = SimulateRequest {
        rpc_url: args.rpc,
        block: parse_block_input(&args.block)?,
        tx: TxInput {
            from: args.from,
            to: args.to,
            data: args.data,
            value: args.value,
        },
        options: SimulationOptions {
            disable_balance_check: args.disable_balance_check,
        },
    };

    let simulation = match simulate(&request).await {
        Ok(result) => result,
        Err(err) => {
            print_error_json(err)?;
            std::process::exit(1);
        }
    };

    let findings = RiskEngine::default().evaluate(&simulation);
    let output = SimulateResponse {
        simulation,
        findings,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&output).context("failed to serialize output json")?
    );
    Ok(())
}

fn parse_block_input(value: &str) -> anyhow::Result<BlockInput> {
    if value.eq_ignore_ascii_case("latest") {
        return Ok(BlockInput::Tag(BlockTag::Latest));
    }
    let number = value.parse::<u64>().with_context(|| {
        format!("invalid --block value '{value}', expected 'latest' or block number")
    })?;
    Ok(BlockInput::Number(number))
}

fn print_error_json(err: SimError) -> anyhow::Result<()> {
    let envelope = match err {
        SimError::BadInput(message) => ErrorEnvelope::new("BAD_REQUEST", message, None),
        SimError::Rpc(message) => ErrorEnvelope::new("RPC_ERROR", message, None),
        SimError::Simulation(message) => ErrorEnvelope::new("SIMULATION_ERROR", message, None),
    };
    let text = serde_json::to_string_pretty(&envelope)
        .map_err(|error| anyhow!("failed to serialize error json: {error}"))?;
    eprintln!("{text}");
    Ok(())
}
