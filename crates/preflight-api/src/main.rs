use std::{net::SocketAddr, sync::Arc};

use anyhow::Context as _;
use axum::{
    Json, Router,
    extract::{Json as JsonExtractor, State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use preflight_risk::RiskEngine;
use preflight_sim::{SimError, simulate};
use preflight_types::{ErrorEnvelope, SimulateRequest, SimulateResponse};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    risk_engine: Arc<RiskEngine>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    body: ErrorEnvelope,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: ErrorEnvelope::new("BAD_REQUEST", message.into(), None),
        }
    }

    fn from_simulation(err: SimError) -> Self {
        match err {
            SimError::BadInput(message) => Self {
                status: StatusCode::BAD_REQUEST,
                body: ErrorEnvelope::new("BAD_REQUEST", message, None),
            },
            SimError::Rpc(message) => Self {
                status: StatusCode::BAD_GATEWAY,
                body: ErrorEnvelope::new("RPC_ERROR", message, None),
            },
            SimError::Simulation(message) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                body: ErrorEnvelope::new("SIMULATION_ERROR", message, None),
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

#[derive(Parser, Debug)]
#[command(name = "preflight-api", about = "EVM Preflight HTTP server")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:3000")]
    bind: SocketAddr,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let args = Args::parse();

    let state = AppState {
        risk_engine: Arc::new(RiskEngine::default()),
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/simulate", post(simulate_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("failed to bind {}", args.bind))?;
    tracing::info!("preflight-api listening on {}", args.bind);

    axum::serve(listener, app)
        .await
        .context("axum server terminated unexpectedly")?;
    Ok(())
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("preflight_api=info,preflight_sim=info")),
        )
        .with(fmt::layer().with_target(false))
        .init();
}

async fn healthz() -> &'static str {
    "ok"
}

async fn simulate_handler(
    State(state): State<AppState>,
    payload: Result<JsonExtractor<SimulateRequest>, JsonRejection>,
) -> Result<Json<SimulateResponse>, ApiError> {
    let JsonExtractor(request) = payload.map_err(|rejection| {
        ApiError::bad_request(format!("invalid JSON payload: {}", rejection.body_text()))
    })?;

    let simulation = simulate(&request)
        .await
        .map_err(ApiError::from_simulation)?;
    let findings = state.risk_engine.evaluate(&simulation);

    Ok(Json(SimulateResponse {
        simulation,
        findings,
    }))
}
