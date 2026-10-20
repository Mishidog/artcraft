use super::cost::{decide, sanitize_fields, video_multiplier, CapDecision};
use super::security::{check_request, generate_token, is_safe_media_token, safe_extension};
use super::settings::{effective_cap, load_or_create, write_private, CONNECTION_FILE};
use crate::core::commands::cost_estimate::estimate_image_cost_command::estimate_image_cost_command;
use crate::core::commands::cost_estimate::estimate_video_cost_command::estimate_video_cost_command;
use crate::core::commands::generate::generate_image::generate_image_command::generate_image_command;
use crate::core::commands::generate::generate_video::generate_video_command::generate_video_command;
use crate::core::commands::generate::models::image::list_image_models_command::list_image_models_command;
use crate::core::commands::generate::models::video::list_video_models_command::list_video_models_command;
use crate::core::commands::generate::omni::{OmniRequest, OmniResult};
use crate::core::commands::task_queue::get_task_queue_command::handle_request as list_tasks;
use crate::core::state::app_env_configs::app_env_configs::AppEnvConfigs;
use crate::core::state::data_dir::app_data_root::AppDataRoot;
use crate::core::state::data_dir::trait_data_subdir::DataSubdir;
use crate::core::state::task_database::TaskDatabase;
use crate::services::storyteller::commands::storyteller_get_credits_command::storyteller_get_credits_command;
use crate::services::storyteller::state::storyteller_credential_manager::StorytellerCredentialManager;
use crate::version::ARTCRAFT_VERSION;
use artcraft_client::endpoints::media_files::get_media_file::get_media_file;
use axum::extract::{ConnectInfo, Path as AxPath, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use enums::common::generation_provider::GenerationProvider;
use log::{error, info, warn};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokens::tokens::media_files::MediaFileToken;

#[derive(Clone)]
struct BridgeState {
  app: AppHandle,
  token: Arc<String>,
  state_dir: PathBuf,
  downloads_dir: PathBuf,
}

#[derive(Clone, Copy)]
enum Kind { Image, Video }

/// Starts the bridge if enabled by env var. Never panics the app on failure.
pub fn maybe_start(app: AppHandle) {
  if !super::enabled_by_env() {
    return;
  }
  tauri::async_runtime::spawn(async move {
    if let Err(err) = start(app).await {
      error!("MCP bridge failed to start: {:?}", err);
    }
  });
}

async fn start(app: AppHandle) -> anyhow::Result<()> {
  let root = app.state::<AppDataRoot>();
  let state_dir = root.state_dir().path().to_path_buf();
  let downloads_dir = root.downloads_dir().path().join("mcp");
  std::fs::create_dir_all(&downloads_dir)?;
  let settings = load_or_create(&state_dir);

  let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
  let port = listener.local_addr()?.port();
  let token = generate_token();
  let conn = json!({ "port": port, "token": token, "pid": std::process::id(), "version": ARTCRAFT_VERSION });
  write_private(&state_dir.join(CONNECTION_FILE), serde_json::to_string_pretty(&conn)?.as_bytes())?;
  info!("MCP bridge listening on 127.0.0.1:{} (cap {} credits/job)", port, settings.max_credits_per_job);

  let state = BridgeState { app, token: Arc::new(token), state_dir, downloads_dir };
  let router = Router::new()
    .route("/v1/status", get(status))
    .route("/v1/models", get(models))
    .route("/v1/estimate", post(estimate))
    .route("/v1/generate/image", post(generate_image))
    .route("/v1/generate/video", post(generate_video))
    .route("/v1/tasks", get(tasks))
    .route("/v1/tasks/{id}", get(task))
    .route("/v1/download/{media}", post(download))
    .layer(middleware::from_fn_with_state(state.clone(), guard))
    .with_state(state);
  axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await?;
  Ok(())
}

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
  (status, Json(json!({ "error": msg.into() }))).into_response()
}

/// Auth / origin / loopback guard. Logs method, path and status only (never bodies).
async fn guard(State(s): State<BridgeState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request, next: Next) -> Response {
  let h = req.headers();
  let get = |n: header::HeaderName| h.get(n).and_then(|v| v.to_str().ok()).map(str::to_owned);
  let (origin, host, auth) = (get(header::ORIGIN), get(header::HOST), get(header::AUTHORIZATION));
  let method = req.method().clone();
  let path = req.uri().path().to_owned();
  if let Err(rej) = check_request(peer, origin.as_deref(), host.as_deref(), auth.as_deref(), &s.token) {
    warn!("MCP bridge rejected {} {}: {:?}", method, path, rej);
    return err(StatusCode::from_u16(rej.status()).unwrap(), rej.message());
  }
  let resp = next.run(req).await;
  info!("MCP bridge {} {} -> {}", method, path, resp.status());
  resp
}

/// Turns an Omni/command result into (ok, payload-or-error JSON).
fn split<T: serde::Serialize, E: serde::Serialize>(r: Result<T, E>) -> Result<Value, Value> {
  match r {
    Ok(v) => Ok(serde_json::to_value(v).ok().and_then(|v| v.get("payload").cloned()).unwrap_or(Value::Null)),
    Err(e) => Err(serde_json::to_value(e).unwrap_or(json!("error"))),
  }
}

async fn status(State(s): State<BridgeState>) -> Response {
  let creds = s.app.state::<StorytellerCredentialManager>();
  // A session cookie means a signed-in account (an avt-only set is an anonymous visitor).
  let logged_in = matches!(creds.get_credentials(), Ok(Some(ref c)) if c.session.is_some());
  let credits = if logged_in {
    split(storyteller_get_credits_command(s.app.state(), s.app.state()).await).ok()
  } else { None };
  let cap = load_or_create(&s.state_dir).max_credits_per_job;
  Json(json!({
    "app_version": ARTCRAFT_VERSION,
    "logged_in": logged_in,
    "credits": credits,
    "max_credits_per_job": cap,
    "downloads_dir": s.downloads_dir,
  })).into_response()
}

#[derive(Deserialize)]
struct ModelsQuery { modality: Option<String> }

async fn models(State(s): State<BridgeState>, Query(q): Query<ModelsQuery>) -> Response {
  let cfg = s.app.state::<AppEnvConfigs>();
  let r = match q.modality.as_deref().unwrap_or("image") {
    "image" => split(list_image_models_command(cfg).await),
    "video" => split(list_video_models_command(cfg).await),
    _ => return err(StatusCode::BAD_REQUEST, "modality must be image or video"),
  };
  match r { Ok(v) => Json(v).into_response(), Err(e) => (StatusCode::BAD_GATEWAY, Json(e)).into_response() }
}

fn to_request(fields: Map<String, Value>) -> OmniRequest {
  // Always bill through the signed-in ArtCraft (Storyteller) account.
  OmniRequest { provider: Some(GenerationProvider::Artcraft), frontend_caller: None, frontend_subscriber_id: None, frontend_subscriber_payload: None, fields }
}

async fn run_estimate(app: &AppHandle, kind: Kind, fields: &Map<String, Value>) -> OmniResult {
  match kind {
    Kind::Image => estimate_image_cost_command(to_request(fields.clone()), app.clone()).await,
    Kind::Video => estimate_video_cost_command(to_request(fields.clone()), app.clone()).await,
  }
}

#[derive(Deserialize)]
struct EstimateBody { modality: String, #[serde(flatten)] fields: Map<String, Value> }

async fn estimate(State(s): State<BridgeState>, Json(body): Json<EstimateBody>) -> Response {
  let kind = match body.modality.as_str() { "image" => Kind::Image, "video" => Kind::Video, _ => return err(StatusCode::BAD_REQUEST, "modality must be image or video") };
  let (fields, req_cap) = sanitize_fields(body.fields);
  let cap = effective_cap(load_or_create(&s.state_dir).max_credits_per_job, req_cap);
  let mult = if matches!(kind, Kind::Video) { video_multiplier(&fields) } else { 1 };
  match split(run_estimate(&s.app, kind, &fields).await) {
    Ok(p) => {
      let d = decide(&p, mult, cap);
      let (est, within) = match d { CapDecision::Allowed { estimated_credits } => (Some(estimated_credits), true), CapDecision::OverCap { estimated_credits, .. } => (Some(estimated_credits), false), CapDecision::UnknownCost => (None, false) };
      Json(json!({ "estimate": p, "estimated_credits": est, "cap": cap, "within_cap": within })).into_response()
    }
    Err(e) => (StatusCode::BAD_GATEWAY, Json(e)).into_response(),
  }
}

async fn generate_image(State(s): State<BridgeState>, Json(b): Json<Map<String, Value>>) -> Response { generate(s, Kind::Image, b).await }
async fn generate_video(State(s): State<BridgeState>, Json(b): Json<Map<String, Value>>) -> Response { generate(s, Kind::Video, b).await }

async fn generate(s: BridgeState, kind: Kind, body: Map<String, Value>) -> Response {
  let (fields, req_cap) = sanitize_fields(body);
  if fields.get("model").and_then(Value::as_str).is_none() {
    return err(StatusCode::BAD_REQUEST, "model is required (see GET /v1/models)");
  }
  let cap = effective_cap(load_or_create(&s.state_dir).max_credits_per_job, req_cap);
  let mult = if matches!(kind, Kind::Video) { video_multiplier(&fields) } else { 1 };
  // 1) Estimate with the app's own estimator.
  let payload = match split(run_estimate(&s.app, kind, &fields).await) {
    Ok(p) => p,
    Err(e) => return (StatusCode::BAD_GATEWAY, Json(json!({ "error": "estimate failed; not generating", "details": e }))).into_response(),
  };
  let estimated = match decide(&payload, mult, cap) {
    CapDecision::Allowed { estimated_credits } => estimated_credits,
    CapDecision::UnknownCost => return err(StatusCode::UNPROCESSABLE_ENTITY, "cost estimate unavailable; refusing to generate"),
    CapDecision::OverCap { estimated_credits, cap } => return (StatusCode::PAYMENT_REQUIRED, Json(json!({
      "error": "estimated cost exceeds per-job credit cap", "estimated_credits": estimated_credits, "cap": cap,
      "hint": "lower the cost (cheaper model/shorter/fewer) or raise max_credits_per_job in ~/Artcraft/state/mcp-bridge-settings.json"
    }))).into_response(),
  };
  // 2) Generate through the app's own command so the task appears in the UI queue.
  let result = match kind {
    Kind::Image => generate_image_command(to_request(fields), s.app.clone()).await,
    Kind::Video => generate_video_command(to_request(fields), s.app.clone()).await,
  };
  match split(result) {
    Ok(p) => {
      let mut job_tokens: Vec<Value> = Vec::new();
      if let Some(t) = p.get("inference_job_token") { job_tokens.push(t.clone()); }
      if let Some(Value::Array(all)) = p.get("all_job_tokens") { for t in all { if !job_tokens.contains(t) { job_tokens.push(t.clone()); } } }
      Json(json!({ "job_tokens": job_tokens, "estimated_credits": estimated, "cap": cap, "response": p })).into_response()
    }
    Err(e) => (StatusCode::BAD_GATEWAY, Json(e)).into_response(),
  }
}

async fn all_tasks(s: &BridgeState) -> Result<Vec<Value>, Response> {
  let db = s.app.try_state::<TaskDatabase>().ok_or_else(|| err(StatusCode::SERVICE_UNAVAILABLE, "task database not ready"))?;
  let items = list_tasks(&db).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{:?}", e)))?;
  Ok(items.into_iter().filter_map(|t| serde_json::to_value(t).ok()).collect())
}

async fn tasks(State(s): State<BridgeState>) -> Response {
  match all_tasks(&s).await { Ok(t) => Json(json!({ "tasks": t })).into_response(), Err(r) => r }
}

async fn task(State(s): State<BridgeState>, AxPath(id): AxPath<String>) -> Response {
  let list = match all_tasks(&s).await { Ok(t) => t, Err(r) => return r };
  let found = list.into_iter().find(|t| t.get("id").and_then(Value::as_str) == Some(&id) || t.get("provider_job_id").and_then(Value::as_str) == Some(&id));
  match found { Some(t) => Json(t).into_response(), None => err(StatusCode::NOT_FOUND, "no task with that id or job token") }
}

async fn download(State(s): State<BridgeState>, AxPath(media): AxPath<String>) -> Response {
  if !is_safe_media_token(&media) { return err(StatusCode::BAD_REQUEST, "invalid media token"); }
  let cfg = s.app.state::<AppEnvConfigs>();
  let token = MediaFileToken::new_from_str(&media);
  let file = match get_media_file(&cfg.storyteller_host, &token).await {
    Ok(f) => f,
    Err(e) => return err(StatusCode::BAD_GATEWAY, format!("media lookup failed: {}", e)),
  };
  let url_str = file.media_file.media_links.cdn_url.to_string();
  let url = match url::Url::parse(&url_str) { Ok(u) if u.scheme() == "https" => u, _ => return err(StatusCode::BAD_GATEWAY, "media has no https CDN url") };
  let bytes = match reqwest::get(url.clone()).await.and_then(|r| r.error_for_status()) {
    Ok(r) => match r.bytes().await { Ok(b) => b, Err(e) => return err(StatusCode::BAD_GATEWAY, e.to_string()) },
    Err(e) => return err(StatusCode::BAD_GATEWAY, e.to_string()),
  };
  let path = s.downloads_dir.join(format!("{}.{}", media, safe_extension(url.path())));
  if let Err(e) = tokio::fs::write(&path, &bytes).await { return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()); }
  Json(json!({ "path": path, "bytes": bytes.len() })).into_response()
}
