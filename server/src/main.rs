use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Form, Path as AxumPath, Query, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, patch, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use rand::{rngs::OsRng, Rng, RngCore};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

const TOKEN_LEN: usize = 128;
const TOKEN_CHARS: &[u8] =
    b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ!@#$%^&*()-_=+[]{}:,.?/";
const ADMIN_SESSION_COOKIE: &str = "guideng_admin_session";
const AUTO_CLEANUP_DAYS_KEY: &str = "admin.auto_cleanup_days";

#[derive(Clone)]
struct AppState {
    token: Arc<String>,
    admin_password: Arc<String>,
    admin_path: Arc<String>,
    admin_session: Arc<String>,
    store: Store,
    map_config: MapConfig,
}

#[derive(Clone)]
struct Store {
    db: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, Serialize)]
struct MapConfig {
    provider: &'static str,
    amap_web_js_api_key: Option<String>,
    amap_web_js_security_code: Option<String>,
    amap_android_key: Option<String>,
    amap_ios_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Device {
    id: Uuid,
    name: String,
    platform: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_location: Option<Location>,
}

#[derive(Debug, Clone, Serialize)]
struct Location {
    id: Option<i64>,
    latitude: f64,
    longitude: f64,
    accuracy: Option<f64>,
    altitude: Option<f64>,
    heading: Option<f64>,
    speed: Option<f64>,
    battery_level: Option<f64>,
    captured_at: DateTime<Utc>,
    received_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct RegisterDeviceRequest {
    id: Option<Uuid>,
    name: String,
    platform: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateDeviceRequest {
    name: Option<String>,
    platform: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocationRequest {
    latitude: f64,
    longitude: f64,
    accuracy: Option<f64>,
    altitude: Option<f64>,
    heading: Option<f64>,
    speed: Option<f64>,
    battery_level: Option<f64>,
    captured_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct TrackQuery {
    days: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct AdminQuery {
    message: Option<String>,
    msg: Option<String>,
    count: Option<usize>,
    lang: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AdminLoginForm {
    password: String,
    lang: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DaysForm {
    days: i64,
    lang: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AutoCleanupForm {
    days: Option<i64>,
    lang: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LangForm {
    lang: Option<String>,
}

#[derive(Debug)]
struct AdminDevice {
    device: Device,
    location_count: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AdminLang {
    Zh,
    En,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _log_guard = setup_logging()?;

    let token = load_or_generate_token();
    let admin_password = load_or_generate_admin_password();
    let admin_path = normalize_admin_path(
        env::var("GUIDENG_ADMIN_PATH").unwrap_or_else(|_| "/admin".to_string()),
    )?;
    let admin_session = generate_session_secret();
    let bind = env::var("GUIDENG_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let database_url =
        env::var("GUIDENG_DATABASE_URL").unwrap_or_else(|_| "/data/guideng.sqlite3".to_string());
    let cors_origins = env::var("GUIDENG_CORS_ORIGINS").unwrap_or_else(|_| "*".to_string());
    let map_config = MapConfig::from_env();

    if env::var("GUIDENG_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some()
    {
        tracing::info!("using GUIDENG_TOKEN from environment");
    } else {
        tracing::warn!("GUIDENG_TOKEN was not set; generated startup token: {token}");
    }
    if env::var("GUIDENG_ADMIN_PASSWORD")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some()
    {
        tracing::info!("using GUIDENG_ADMIN_PASSWORD from environment");
    } else {
        tracing::warn!(
            "GUIDENG_ADMIN_PASSWORD was not set; generated startup admin password: {admin_password}"
        );
    }

    let store = Store::open(PathBuf::from(database_url))?;
    let state = AppState {
        token: Arc::new(token),
        admin_password: Arc::new(admin_password),
        admin_path: Arc::new(admin_path),
        admin_session: Arc::new(admin_session),
        store,
        map_config,
    };

    let api = Router::new()
        .route("/config", get(get_config))
        .route("/devices", get(list_devices).post(register_device))
        .route("/devices/:id", patch(update_device))
        .route("/devices/:id/location", post(update_location))
        .route("/devices/:id/tracks", get(list_tracks))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));

    let admin = Router::new()
        .route("/", get(admin_index))
        .route("/login", get(admin_login_page).post(admin_login))
        .route("/logout", post(admin_logout))
        .route(
            "/devices/:id/locations/delete",
            post(admin_delete_locations),
        )
        .route("/devices/:id/delete", post(admin_delete_device))
        .route("/stale/delete", post(admin_delete_stale))
        .route("/auto-cleanup", post(admin_set_auto_cleanup));

    spawn_auto_cleanup(state.clone());

    let admin_path = state.admin_path.as_str().to_string();
    let app = Router::new()
        .route("/health", get(health))
        .nest("/api", api)
        .nest(&admin_path, admin)
        .layer(cors_layer(&cors_origins)?)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = bind.parse().context("invalid GUIDENG_BIND")?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("guideng server listening on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn setup_logging() -> Result<tracing_appender::non_blocking::WorkerGuard> {
    let log_path = default_log_path()?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let log_dir = log_path
        .parent()
        .ok_or_else(|| anyhow!("invalid GUIDENG_LOG_PATH"))?;
    let log_file = log_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid GUIDENG_LOG_PATH"))?;
    let file_appender = tracing_appender::rolling::never(log_dir, log_file);
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "guideng_server=info,tower_http=info".into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(file_writer)
                .with_ansi(false),
        )
        .init();

    tracing::info!("log file: {}", log_path.display());
    Ok(guard)
}

fn default_log_path() -> Result<PathBuf> {
    if let Ok(path) = env::var("GUIDENG_LOG_PATH") {
        let path = path.trim();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }

    let cwd = env::current_dir()?;
    if cwd.file_name().and_then(|name| name.to_str()) == Some("server") {
        return Ok(cwd.join("guideng.log"));
    }

    let server_dir = cwd.join("server");
    if Path::new(&server_dir).is_dir() {
        return Ok(server_dir.join("guideng.log"));
    }

    Ok(cwd.join("guideng.log"))
}

fn load_or_generate_token() -> String {
    env::var("GUIDENG_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(generate_token)
}

fn load_or_generate_admin_password() -> String {
    env::var("GUIDENG_ADMIN_PASSWORD")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(generate_token)
}

fn generate_token() -> String {
    let mut rng = OsRng;
    (0..TOKEN_LEN)
        .map(|_| {
            let index = rng.gen_range(0..TOKEN_CHARS.len());
            TOKEN_CHARS[index] as char
        })
        .collect()
}

fn generate_session_secret() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn normalize_admin_path(path: String) -> Result<String> {
    let path = path.trim();
    if path.is_empty() {
        return Ok("/admin".to_string());
    }
    if path.contains(['?', '#'])
        || path.split('/').any(|part| part == "." || part == "..")
        || !path
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.' | '~'))
    {
        return Err(anyhow!("invalid GUIDENG_ADMIN_PATH"));
    }

    let mut normalized = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    if normalized == "/" || normalized == "/api" || normalized == "/health" {
        return Err(anyhow!(
            "GUIDENG_ADMIN_PATH conflicts with a reserved route"
        ));
    }
    Ok(normalized)
}

impl MapConfig {
    fn from_env() -> Self {
        Self {
            provider: "amap",
            amap_web_js_api_key: read_optional_env("GUIDENG_AMAP_WEB_JS_API_KEY"),
            amap_web_js_security_code: read_optional_env("GUIDENG_AMAP_WEB_JS_SECURITY_CODE"),
            amap_android_key: read_optional_env("GUIDENG_AMAP_ANDROID_KEY"),
            amap_ios_key: read_optional_env("GUIDENG_AMAP_IOS_KEY"),
        }
    }
}

fn read_optional_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn cors_layer(origins: &str) -> Result<CorsLayer> {
    let layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderNameExt::guideng_token(),
        ]);

    if origins.trim() == "*" {
        Ok(layer.allow_origin(tower_http::cors::Any))
    } else {
        let values = origins
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(HeaderValue::from_str)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(layer.allow_origin(values))
    }
}

struct HeaderNameExt;

impl HeaderNameExt {
    fn guideng_token() -> header::HeaderName {
        header::HeaderName::from_static("x-guideng-token")
    }
}

async fn auth(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let custom = headers
        .get(HeaderNameExt::guideng_token())
        .and_then(|value| value.to_str().ok());
    let provided = bearer.or(custom);

    if provided == Some(state.token.as_str()) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse::new("invalid token")),
        )
            .into_response()
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "name": "guideng" }))
}

async fn get_config(State(state): State<AppState>) -> Json<MapConfig> {
    Json(state.map_config.clone())
}

async fn list_devices(State(state): State<AppState>) -> Result<Json<Vec<Device>>, ApiError> {
    Ok(Json(state.store.list_devices()?))
}

async fn register_device(
    State(state): State<AppState>,
    Json(payload): Json<RegisterDeviceRequest>,
) -> Result<Json<Device>, ApiError> {
    validate_name(&payload.name)?;
    let id = payload.id.unwrap_or_else(Uuid::new_v4);
    let device = state
        .store
        .upsert_device(id, payload.name.trim(), payload.platform)?;
    Ok(Json(device))
}

async fn update_device(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<Uuid>,
    Json(payload): Json<UpdateDeviceRequest>,
) -> Result<Json<Device>, ApiError> {
    if let Some(name) = &payload.name {
        validate_name(name)?;
    }
    let device = state.store.update_device(id, payload)?;
    Ok(Json(device))
}

async fn update_location(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<Uuid>,
    Json(payload): Json<LocationRequest>,
) -> Result<Json<Device>, ApiError> {
    validate_location(&payload)?;
    let location = Location {
        id: None,
        latitude: payload.latitude,
        longitude: payload.longitude,
        accuracy: payload.accuracy,
        altitude: payload.altitude,
        heading: payload.heading,
        speed: payload.speed,
        battery_level: payload.battery_level,
        captured_at: payload.captured_at.unwrap_or_else(Utc::now),
        received_at: Utc::now(),
    };
    let device = state.store.insert_location(id, location)?;
    run_auto_cleanup(&state);
    Ok(Json(device))
}

async fn list_tracks(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<Uuid>,
    Query(query): Query<TrackQuery>,
) -> Result<Json<Vec<Location>>, ApiError> {
    let days = query.days.unwrap_or(7).clamp(1, 7);
    Ok(Json(state.store.list_tracks(id, days)?))
}

async fn admin_index(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdminQuery>,
) -> Result<Response, ApiError> {
    let lang = AdminLang::from_option(query.lang.as_deref());
    if !is_admin_authenticated(&state, &headers) {
        return Ok(
            Redirect::to(&admin_url(&state, &format!("/login?lang={}", lang.code())))
                .into_response(),
        );
    }

    let devices = state.store.list_admin_devices()?;
    let auto_cleanup_days = state.store.auto_cleanup_days()?;
    let message = admin_notice(&query, lang);
    Ok(Html(render_admin_page(
        &state,
        &devices,
        auto_cleanup_days,
        message.as_deref(),
        lang,
    ))
    .into_response())
}

async fn admin_login_page(
    State(state): State<AppState>,
    Query(query): Query<AdminQuery>,
) -> Html<String> {
    Html(render_login_page(
        &state,
        None,
        AdminLang::from_option(query.lang.as_deref()),
    ))
}

async fn admin_login(
    State(state): State<AppState>,
    Form(payload): Form<AdminLoginForm>,
) -> Result<Response, ApiError> {
    let lang = AdminLang::from_option(payload.lang.as_deref());
    if payload.password == *state.admin_password {
        let cookie = format!(
            "{ADMIN_SESSION_COOKIE}={}; HttpOnly; SameSite=Lax; Path={}; Max-Age=2592000",
            state.admin_session,
            state.admin_path.as_str()
        );
        return Ok((
            [(header::SET_COOKIE, cookie)],
            Redirect::to(&admin_url(&state, &format!("?lang={}", lang.code()))),
        )
            .into_response());
    }

    Ok((
        StatusCode::UNAUTHORIZED,
        Html(render_login_page(&state, Some(lang.login_error()), lang)),
    )
        .into_response())
}

async fn admin_logout(State(state): State<AppState>, Form(payload): Form<LangForm>) -> Response {
    let lang = AdminLang::from_option(payload.lang.as_deref());
    let cookie = format!(
        "{ADMIN_SESSION_COOKIE}=; HttpOnly; SameSite=Lax; Path={}; Max-Age=0",
        state.admin_path.as_str()
    );
    (
        [(header::SET_COOKIE, cookie)],
        Redirect::to(&admin_url(&state, &format!("/login?lang={}", lang.code()))),
    )
        .into_response()
}

async fn admin_delete_locations(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<Uuid>,
    Form(payload): Form<LangForm>,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    let deleted = state.store.delete_device_locations(id)?;
    Ok(Redirect::to(&admin_notice_url(
        &state,
        AdminLang::from_option(payload.lang.as_deref()),
        "location_records_deleted",
        Some(deleted),
    ))
    .into_response())
}

async fn admin_delete_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<Uuid>,
    Form(payload): Form<LangForm>,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    state.store.delete_device(id)?;
    Ok(Redirect::to(&admin_notice_url(
        &state,
        AdminLang::from_option(payload.lang.as_deref()),
        "device_deleted",
        None,
    ))
    .into_response())
}

async fn admin_delete_stale(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<DaysForm>,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    let lang = AdminLang::from_option(payload.lang.as_deref());
    let days = validate_days(payload.days)?;
    let deleted = state.store.delete_stale_devices(days)?;
    Ok(Redirect::to(&admin_notice_url(
        &state,
        lang,
        "stale_devices_deleted",
        Some(deleted),
    ))
    .into_response())
}

async fn admin_set_auto_cleanup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<AutoCleanupForm>,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    let lang = AdminLang::from_option(payload.lang.as_deref());
    let days = match payload.days {
        Some(days) if days > 0 => Some(validate_days(days)?),
        _ => None,
    };
    state.store.set_auto_cleanup_days(days)?;
    if let Some(days) = days {
        let deleted = state.store.delete_stale_devices(days)?;
        Ok(Redirect::to(&admin_notice_url(
            &state,
            lang,
            "auto_cleanup_saved",
            Some(deleted),
        ))
        .into_response())
    } else {
        Ok(Redirect::to(&admin_notice_url(
            &state,
            lang,
            "auto_cleanup_disabled",
            None,
        ))
        .into_response())
    }
}

fn is_admin_authenticated(state: &AppState, headers: &HeaderMap) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| cookie_value(value, ADMIN_SESSION_COOKIE))
        .is_some_and(|value| value == state.admin_session.as_str())
}

fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if is_admin_authenticated(state, headers) {
        Ok(())
    } else {
        Err(ApiError::unauthorized("admin login required"))
    }
}

fn cookie_value<'a>(cookies: &'a str, name: &str) -> Option<&'a str> {
    cookies.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then_some(value)
    })
}

fn admin_url(state: &AppState, suffix: &str) -> String {
    if suffix.is_empty() || suffix == "/" {
        state.admin_path.as_str().to_string()
    } else if suffix.starts_with('/') || suffix.starts_with('?') {
        format!("{}{}", state.admin_path, suffix)
    } else {
        format!("{}/{}", state.admin_path, suffix)
    }
}

fn admin_notice_url(
    state: &AppState,
    lang: AdminLang,
    message: &str,
    count: Option<usize>,
) -> String {
    let mut suffix = format!("?lang={}&msg={message}", lang.code());
    if let Some(count) = count {
        suffix.push_str(&format!("&count={count}"));
    }
    admin_url(state, &suffix)
}

fn admin_notice(query: &AdminQuery, lang: AdminLang) -> Option<String> {
    if let Some(message) = query.message.as_deref() {
        return Some(message.to_string());
    }

    let count = query.count.unwrap_or_default();
    query.msg.as_deref().map(|message| match (lang, message) {
        (AdminLang::Zh, "location_records_deleted") => format!("{count} 条位置记录已删除"),
        (AdminLang::En, "location_records_deleted") => format!("{count} location records deleted"),
        (AdminLang::Zh, "device_deleted") => "客户端已删除".to_string(),
        (AdminLang::En, "device_deleted") => "Device deleted".to_string(),
        (AdminLang::Zh, "stale_devices_deleted") => format!("{count} 个未更新客户端已删除"),
        (AdminLang::En, "stale_devices_deleted") => format!("{count} stale devices deleted"),
        (AdminLang::Zh, "auto_cleanup_saved") => {
            format!("自动清理设置已保存，已删除 {count} 个客户端")
        }
        (AdminLang::En, "auto_cleanup_saved") => {
            format!("Auto cleanup saved, {count} devices deleted")
        }
        (AdminLang::Zh, "auto_cleanup_disabled") => "自动清理已关闭".to_string(),
        (AdminLang::En, "auto_cleanup_disabled") => "Auto cleanup disabled".to_string(),
        (_, fallback) => fallback.to_string(),
    })
}

fn hidden_lang_input(lang: AdminLang) -> String {
    format!(
        r#"<input type="hidden" name="lang" value="{}">"#,
        lang.code()
    )
}

impl AdminLang {
    fn from_option(value: Option<&str>) -> Self {
        match value {
            Some("en") => Self::En,
            _ => Self::Zh,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }

    fn html_lang(self) -> &'static str {
        match self {
            Self::Zh => "zh-CN",
            Self::En => "en",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Zh => "Guideng 管理后台",
            Self::En => "Guideng Admin",
        }
    }

    fn switch_label(self) -> &'static str {
        match self {
            Self::Zh => "English",
            Self::En => "中文",
        }
    }

    fn switch_code(self) -> &'static str {
        match self {
            Self::Zh => "en",
            Self::En => "zh",
        }
    }

    fn password_label(self) -> &'static str {
        match self {
            Self::Zh => "管理密码",
            Self::En => "Admin password",
        }
    }

    fn login(self) -> &'static str {
        match self {
            Self::Zh => "登录",
            Self::En => "Log in",
        }
    }

    fn login_error(self) -> &'static str {
        match self {
            Self::Zh => "密码不正确",
            Self::En => "Incorrect password",
        }
    }

    fn logout(self) -> &'static str {
        match self {
            Self::Zh => "退出",
            Self::En => "Log out",
        }
    }

    fn devices(self) -> &'static str {
        match self {
            Self::Zh => "客户端",
            Self::En => "Devices",
        }
    }

    fn empty_devices(self) -> &'static str {
        match self {
            Self::Zh => "暂无客户端",
            Self::En => "No devices",
        }
    }

    fn manual_cleanup(self) -> &'static str {
        match self {
            Self::Zh => "手动清理未更新客户端",
            Self::En => "Manual cleanup of inactive devices",
        }
    }

    fn auto_cleanup(self) -> &'static str {
        match self {
            Self::Zh => "自动清理设置",
            Self::En => "Auto cleanup settings",
        }
    }

    fn days(self) -> &'static str {
        match self {
            Self::Zh => "天数",
            Self::En => "Days",
        }
    }

    fn delete_now(self) -> &'static str {
        match self {
            Self::Zh => "立即删除",
            Self::En => "Delete now",
        }
    }

    fn save_settings(self) -> &'static str {
        match self {
            Self::Zh => "保存设置",
            Self::En => "Save settings",
        }
    }

    fn empty_to_disable(self) -> &'static str {
        match self {
            Self::Zh => "留空关闭",
            Self::En => "Leave empty to disable",
        }
    }

    fn auto_status(self, days: Option<i64>) -> String {
        match (self, days) {
            (Self::Zh, Some(days)) => format!("当前：自动删除超过 {days} 天未更新位置的客户端"),
            (Self::En, Some(days)) => {
                format!("Current: automatically delete devices inactive for over {days} days")
            }
            (Self::Zh, None) => "当前：未开启自动清理".to_string(),
            (Self::En, None) => "Current: auto cleanup is disabled".to_string(),
        }
    }

    fn table_headers(self) -> [&'static str; 7] {
        match self {
            Self::Zh => [
                "名称",
                "平台",
                "最后更新",
                "最后位置",
                "位置记录",
                "创建时间",
                "操作",
            ],
            Self::En => [
                "Name",
                "Platform",
                "Last update",
                "Last location",
                "Location records",
                "Created",
                "Actions",
            ],
        }
    }

    fn delete_locations(self) -> &'static str {
        match self {
            Self::Zh => "删除位置",
            Self::En => "Delete locations",
        }
    }

    fn delete_device(self) -> &'static str {
        match self {
            Self::Zh => "删除客户端",
            Self::En => "Delete device",
        }
    }
}

fn validate_days(days: i64) -> Result<i64, ApiError> {
    if !(1..=36500).contains(&days) {
        return Err(ApiError::bad_request("days must be between 1 and 36500"));
    }
    Ok(days)
}

fn render_login_page(state: &AppState, error: Option<&str>, lang: AdminLang) -> String {
    let error_html = error
        .map(|message| format!(r#"<p class="error">{}</p>"#, escape_html(message)))
        .unwrap_or_default();
    let switch_url = admin_url(state, &format!("/login?lang={}", lang.switch_code()));
    format!(
        r#"<!doctype html>
<html lang="{}">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{}</title>
  <style>{}</style>
</head>
<body>
  <main class="login">
    <div class="login-tools"><a class="button secondary" href="{}">{}</a></div>
    <h1>{}</h1>
    <form method="post" action="{}/login">
      {}
      <label>{}<input name="password" type="password" autocomplete="current-password" autofocus required></label>
      {}
      <button type="submit">{}</button>
    </form>
  </main>
</body>
</html>"#,
        lang.html_lang(),
        lang.title(),
        admin_css(),
        switch_url,
        lang.switch_label(),
        lang.title(),
        state.admin_path,
        hidden_lang_input(lang),
        lang.password_label(),
        error_html,
        lang.login()
    )
}

fn render_admin_page(
    state: &AppState,
    devices: &[AdminDevice],
    auto_cleanup_days: Option<i64>,
    message: Option<&str>,
    lang: AdminLang,
) -> String {
    let rows = if devices.is_empty() {
        format!(
            r#"<tr><td colspan="7" class="muted">{}</td></tr>"#,
            lang.empty_devices()
        )
    } else {
        devices
            .iter()
            .map(|entry| render_admin_device_row(state, entry, lang))
            .collect::<Vec<_>>()
            .join("")
    };
    let auto_days_value = auto_cleanup_days
        .map(|days| days.to_string())
        .unwrap_or_default();
    let status = lang.auto_status(auto_cleanup_days);
    let message_html = message
        .map(|value| format!(r#"<p class="notice">{}</p>"#, escape_html(value)))
        .unwrap_or_default();
    let switch_url = admin_url(state, &format!("?lang={}", lang.switch_code()));
    let headers = lang.table_headers();
    let header_html = headers
        .iter()
        .map(|header| format!("<th>{}</th>", escape_html(header)))
        .collect::<Vec<_>>()
        .join("");
    let hidden_lang = hidden_lang_input(lang);

    format!(
        r#"<!doctype html>
<html lang="{}">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{}</title>
  <style>{}</style>
</head>
<body>
  <main>
    <header>
      <div>
        <h1>{}</h1>
        <p>{}</p>
      </div>
      <div class="header-actions">
        <a class="button secondary" href="{}">{}</a>
        <form method="post" action="{}/logout">{}<button type="submit" class="secondary">{}</button></form>
      </div>
    </header>
    {}
    <section>
      <h2>{}</h2>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>{}</tr>
          </thead>
          <tbody>{}</tbody>
        </table>
      </div>
    </section>
    <section class="grid">
      <form method="post" action="{}/stale/delete">
        {}
        <h2>{}</h2>
        <label>{}<input name="days" type="number" min="1" max="36500" value="30" required></label>
        <button type="submit">{}</button>
      </form>
      <form method="post" action="{}/auto-cleanup">
        {}
        <h2>{}</h2>
        <p class="muted">{}</p>
        <label>{}<input name="days" type="number" min="1" max="36500" value="{}" placeholder="{}"></label>
        <button type="submit">{}</button>
      </form>
    </section>
  </main>
</body>
</html>"#,
        lang.html_lang(),
        lang.title(),
        admin_css(),
        lang.title(),
        escape_html(&status),
        switch_url,
        lang.switch_label(),
        state.admin_path,
        hidden_lang,
        lang.logout(),
        message_html,
        lang.devices(),
        header_html,
        rows,
        state.admin_path,
        hidden_lang_input(lang),
        lang.manual_cleanup(),
        lang.days(),
        lang.delete_now(),
        state.admin_path,
        hidden_lang_input(lang),
        lang.auto_cleanup(),
        escape_html(&status),
        lang.days(),
        auto_days_value,
        lang.empty_to_disable(),
        lang.save_settings()
    )
}

fn render_admin_device_row(state: &AppState, entry: &AdminDevice, lang: AdminLang) -> String {
    let device = &entry.device;
    let last_location = device
        .last_location
        .as_ref()
        .map(|location| format!("{:.6}, {:.6}", location.latitude, location.longitude))
        .unwrap_or_else(|| "-".to_string());
    format!(
        r#"<tr>
  <td><strong>{}</strong><br><span class="muted">{}</span></td>
  <td>{}</td>
  <td>{}</td>
  <td>{}</td>
  <td>{}</td>
  <td>{}</td>
  <td class="actions">
    <form method="post" action="{}/devices/{}/locations/delete">{}<button type="submit" class="secondary">{}</button></form>
    <form method="post" action="{}/devices/{}/delete">{}<button type="submit" class="danger">{}</button></form>
  </td>
</tr>"#,
        escape_html(&device.name),
        device.id,
        escape_html(device.platform.as_deref().unwrap_or("-")),
        escape_html(&time_to_text(device.updated_at)),
        escape_html(&last_location),
        entry.location_count,
        escape_html(&time_to_text(device.created_at)),
        state.admin_path,
        device.id,
        hidden_lang_input(lang),
        lang.delete_locations(),
        state.admin_path,
        device.id,
        hidden_lang_input(lang),
        lang.delete_device()
    )
}

fn admin_css() -> &'static str {
    r#"
body{margin:0;background:#f6f7f9;color:#17202a;font:15px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}
main{max-width:1180px;margin:0 auto;padding:28px}
.login{max-width:420px;padding-top:12vh}
header{display:flex;align-items:center;justify-content:space-between;gap:16px;margin-bottom:20px}
h1{font-size:28px;margin:0 0 4px}
h2{font-size:18px;margin:0 0 14px}
p{margin:0;color:#5e6a76}
section,form.login{background:#fff;border:1px solid #dfe4ea;border-radius:8px;padding:18px;margin-bottom:18px}
.login form,.grid form{background:#fff;border:1px solid #dfe4ea;border-radius:8px;padding:18px}
.login-tools{display:flex;justify-content:flex-end;margin-bottom:14px}
.header-actions{display:flex;align-items:center;gap:8px}
.header-actions form{margin:0}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:16px;background:transparent;border:0;padding:0}
label{display:grid;gap:8px;margin-bottom:14px;font-weight:600}
input{box-sizing:border-box;width:100%;border:1px solid #c9d1d9;border-radius:6px;padding:10px;font:inherit}
a.button,button{border:0;border-radius:6px;background:#1f6feb;color:#fff;padding:9px 14px;font:inherit;font-weight:700;cursor:pointer;text-decoration:none;display:inline-flex;align-items:center;justify-content:center}
a.button.secondary,button.secondary{background:#eef2f6;color:#17202a}
button.danger{background:#c93c37}
.table-wrap{overflow:auto}
table{border-collapse:collapse;width:100%;min-width:900px}
th,td{border-bottom:1px solid #e7ebef;padding:10px;text-align:left;vertical-align:top}
th{font-size:13px;color:#5e6a76;background:#fbfcfd}
.actions{display:flex;gap:8px;white-space:nowrap}
.actions form{border:0;padding:0;margin:0;background:transparent}
.muted{color:#6b7785}
.error{color:#b42318;margin-bottom:14px}
.notice{background:#e8f3ff;border:1px solid #b9d9ff;border-radius:6px;color:#174a7c;margin-bottom:16px;padding:10px}
@media (max-width:700px){main{padding:18px}header{align-items:flex-start;flex-direction:column}.header-actions{width:100%;justify-content:space-between}.actions{flex-direction:column}}
"#
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn spawn_auto_cleanup(state: AppState) {
    tokio::spawn(async move {
        run_auto_cleanup(&state);
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60 * 60 * 24));
        loop {
            interval.tick().await;
            run_auto_cleanup(&state);
        }
    });
}

fn run_auto_cleanup(state: &AppState) {
    match state.store.auto_cleanup_days() {
        Ok(Some(days)) => match state.store.delete_stale_devices(days) {
            Ok(deleted) if deleted > 0 => {
                tracing::info!("auto cleanup deleted {deleted} stale devices")
            }
            Ok(_) => {}
            Err(error) => tracing::error!("auto cleanup failed: {}", error.message),
        },
        Ok(None) => {}
        Err(error) => tracing::error!("auto cleanup setting failed: {}", error.message),
    }
}

fn validate_name(name: &str) -> Result<(), ApiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(ApiError::bad_request("device name must be 1-64 characters"));
    }
    Ok(())
}

fn validate_location(payload: &LocationRequest) -> Result<(), ApiError> {
    if !(-90.0..=90.0).contains(&payload.latitude) || !(-180.0..=180.0).contains(&payload.longitude)
    {
        return Err(ApiError::bad_request("invalid coordinates"));
    }
    Ok(())
}

impl Store {
    fn open(db_path: PathBuf) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS devices (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                platform TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS locations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                device_id TEXT NOT NULL,
                latitude REAL NOT NULL,
                longitude REAL NOT NULL,
                accuracy REAL,
                altitude REAL,
                heading REAL,
                speed REAL,
                battery_level REAL,
                captured_at TEXT NOT NULL,
                received_at TEXT NOT NULL,
                FOREIGN KEY(device_id) REFERENCES devices(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_locations_device_received
                ON locations(device_id, received_at DESC);

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            "#,
        )?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    fn list_devices(&self) -> Result<Vec<Device>, ApiError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            r#"
            SELECT
                d.id, d.name, d.platform, d.created_at, d.updated_at,
                l.id, l.latitude, l.longitude, l.accuracy, l.altitude, l.heading,
                l.speed, l.battery_level, l.captured_at, l.received_at
            FROM devices d
            LEFT JOIN locations l ON l.id = (
                SELECT id FROM locations
                WHERE device_id = d.id
                ORDER BY received_at DESC, id DESC
                LIMIT 1
            )
            ORDER BY d.updated_at DESC
            "#,
        )?;
        let rows = statement.query_map([], row_to_device)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(ApiError::from)
    }

    fn upsert_device(
        &self,
        id: Uuid,
        name: &str,
        platform: Option<String>,
    ) -> Result<Device, ApiError> {
        let conn = self.conn()?;
        let now = Utc::now();
        conn.execute(
            r#"
            INSERT INTO devices (id, name, platform, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?4)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                platform = excluded.platform,
                updated_at = excluded.updated_at
            "#,
            params![id.to_string(), name, platform, time_to_text(now)],
        )?;
        self.device_by_id_locked(&conn, id)
    }

    fn update_device(&self, id: Uuid, payload: UpdateDeviceRequest) -> Result<Device, ApiError> {
        let conn = self.conn()?;
        let current = self.device_by_id_locked(&conn, id)?;
        let name = payload.name.unwrap_or(current.name);
        let platform = payload.platform.or(current.platform);
        conn.execute(
            "UPDATE devices SET name = ?1, platform = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                name.trim(),
                platform,
                time_to_text(Utc::now()),
                id.to_string()
            ],
        )?;
        self.device_by_id_locked(&conn, id)
    }

    fn insert_location(&self, id: Uuid, location: Location) -> Result<Device, ApiError> {
        let conn = self.conn()?;
        let tx = conn.unchecked_transaction()?;
        let exists: Option<String> = tx
            .query_row(
                "SELECT id FROM devices WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(ApiError::not_found("device not found"));
        }

        let now = Utc::now();
        tx.execute(
            r#"
            INSERT INTO locations (
                device_id, latitude, longitude, accuracy, altitude, heading, speed,
                battery_level, captured_at, received_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                id.to_string(),
                location.latitude,
                location.longitude,
                location.accuracy,
                location.altitude,
                location.heading,
                location.speed,
                location.battery_level,
                time_to_text(location.captured_at),
                time_to_text(location.received_at),
            ],
        )?;
        tx.execute(
            "UPDATE devices SET updated_at = ?1 WHERE id = ?2",
            params![time_to_text(now), id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM locations WHERE received_at < ?1",
            params![time_to_text(Utc::now() - Duration::days(7))],
        )?;
        tx.commit()?;
        self.device_by_id_locked(&conn, id)
    }

    fn list_admin_devices(&self) -> Result<Vec<AdminDevice>, ApiError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            r#"
            SELECT
                d.id, d.name, d.platform, d.created_at, d.updated_at,
                l.id, l.latitude, l.longitude, l.accuracy, l.altitude, l.heading,
                l.speed, l.battery_level, l.captured_at, l.received_at,
                COUNT(all_l.id) AS location_count
            FROM devices d
            LEFT JOIN locations l ON l.id = (
                SELECT id FROM locations
                WHERE device_id = d.id
                ORDER BY received_at DESC, id DESC
                LIMIT 1
            )
            LEFT JOIN locations all_l ON all_l.device_id = d.id
            GROUP BY
                d.id, d.name, d.platform, d.created_at, d.updated_at,
                l.id, l.latitude, l.longitude, l.accuracy, l.altitude, l.heading,
                l.speed, l.battery_level, l.captured_at, l.received_at
            ORDER BY d.updated_at DESC
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok(AdminDevice {
                device: row_to_device(row)?,
                location_count: row.get(15)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(ApiError::from)
    }

    fn delete_device_locations(&self, id: Uuid) -> Result<usize, ApiError> {
        let conn = self.conn()?;
        let exists: Option<String> = conn
            .query_row(
                "SELECT id FROM devices WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(ApiError::not_found("device not found"));
        }
        conn.execute(
            "DELETE FROM locations WHERE device_id = ?1",
            params![id.to_string()],
        )
        .map_err(ApiError::from)
    }

    fn delete_device(&self, id: Uuid) -> Result<usize, ApiError> {
        let conn = self.conn()?;
        let deleted = conn.execute("DELETE FROM devices WHERE id = ?1", params![id.to_string()])?;
        if deleted == 0 {
            return Err(ApiError::not_found("device not found"));
        }
        Ok(deleted)
    }

    fn delete_stale_devices(&self, days: i64) -> Result<usize, ApiError> {
        let conn = self.conn()?;
        let cutoff = Utc::now() - Duration::days(days);
        conn.execute(
            "DELETE FROM devices WHERE updated_at < ?1",
            params![time_to_text(cutoff)],
        )
        .map_err(ApiError::from)
    }

    fn auto_cleanup_days(&self) -> Result<Option<i64>, ApiError> {
        let conn = self.conn()?;
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![AUTO_CLEANUP_DAYS_KEY],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|value| {
                value
                    .parse::<i64>()
                    .map_err(|_| ApiError::internal("invalid auto cleanup setting"))
            })
            .transpose()
    }

    fn set_auto_cleanup_days(&self, days: Option<i64>) -> Result<(), ApiError> {
        let conn = self.conn()?;
        if let Some(days) = days {
            conn.execute(
                r#"
                INSERT INTO settings (key, value)
                VALUES (?1, ?2)
                ON CONFLICT(key) DO UPDATE SET value = excluded.value
                "#,
                params![AUTO_CLEANUP_DAYS_KEY, days.to_string()],
            )?;
        } else {
            conn.execute(
                "DELETE FROM settings WHERE key = ?1",
                params![AUTO_CLEANUP_DAYS_KEY],
            )?;
        }
        Ok(())
    }

    fn list_tracks(&self, id: Uuid, days: i64) -> Result<Vec<Location>, ApiError> {
        let conn = self.conn()?;
        let exists: Option<String> = conn
            .query_row(
                "SELECT id FROM devices WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(ApiError::not_found("device not found"));
        }

        let since = Utc::now() - Duration::days(days);
        let mut statement = conn.prepare(
            r#"
            SELECT id, latitude, longitude, accuracy, altitude, heading, speed,
                   battery_level, captured_at, received_at
            FROM locations
            WHERE device_id = ?1 AND received_at >= ?2
            ORDER BY received_at ASC, id ASC
            "#,
        )?;
        let rows = statement.query_map(params![id.to_string(), time_to_text(since)], |row| {
            Ok(Location {
                id: Some(row.get(0)?),
                latitude: row.get(1)?,
                longitude: row.get(2)?,
                accuracy: row.get(3)?,
                altitude: row.get(4)?,
                heading: row.get(5)?,
                speed: row.get(6)?,
                battery_level: row.get(7)?,
                captured_at: text_to_time(row.get::<_, String>(8)?)?,
                received_at: text_to_time(row.get::<_, String>(9)?)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(ApiError::from)
    }

    fn device_by_id_locked(&self, conn: &Connection, id: Uuid) -> Result<Device, ApiError> {
        conn.query_row(
            r#"
            SELECT
                d.id, d.name, d.platform, d.created_at, d.updated_at,
                l.id, l.latitude, l.longitude, l.accuracy, l.altitude, l.heading,
                l.speed, l.battery_level, l.captured_at, l.received_at
            FROM devices d
            LEFT JOIN locations l ON l.id = (
                SELECT id FROM locations
                WHERE device_id = d.id
                ORDER BY received_at DESC, id DESC
                LIMIT 1
            )
            WHERE d.id = ?1
            "#,
            params![id.to_string()],
            row_to_device,
        )
        .optional()?
        .ok_or(ApiError::not_found("device not found"))
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, ApiError> {
        self.db
            .lock()
            .map_err(|_| ApiError::internal("database lock is poisoned"))
    }
}

fn row_to_device(row: &rusqlite::Row<'_>) -> rusqlite::Result<Device> {
    let location_id: Option<i64> = row.get(5)?;
    let last_location = if let Some(location_id) = location_id {
        Some(Location {
            id: Some(location_id),
            latitude: row.get(6)?,
            longitude: row.get(7)?,
            accuracy: row.get(8)?,
            altitude: row.get(9)?,
            heading: row.get(10)?,
            speed: row.get(11)?,
            battery_level: row.get(12)?,
            captured_at: text_to_time(row.get::<_, String>(13)?)?,
            received_at: text_to_time(row.get::<_, String>(14)?)?,
        })
    } else {
        None
    };

    Ok(Device {
        id: Uuid::parse_str(&row.get::<_, String>(0)?).map_err(parse_error)?,
        name: row.get(1)?,
        platform: row.get(2)?,
        created_at: text_to_time(row.get::<_, String>(3)?)?,
        updated_at: text_to_time(row.get::<_, String>(4)?)?,
        last_location,
    })
}

fn time_to_text(value: DateTime<Utc>) -> String {
    value.to_rfc3339()
}

fn text_to_time(value: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(parse_error)
}

fn parse_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

impl ErrorResponse {
    fn new(message: impl Into<String>) -> Self {
        Self {
            error: message.into(),
        }
    }
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorResponse::new(self.message))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        anyhow::Error::from(error).into()
    }
}

impl From<std::io::Error> for ApiError {
    fn from(error: std::io::Error) -> Self {
        anyhow::Error::from(error).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(admin_path: &str) -> AppState {
        let db_path = std::env::temp_dir().join(format!("guideng-test-{}.sqlite3", Uuid::new_v4()));
        AppState {
            token: Arc::new("test-token".to_string()),
            admin_password: Arc::new("test-password".to_string()),
            admin_path: Arc::new(admin_path.to_string()),
            admin_session: Arc::new("test-session".to_string()),
            store: Store::open(db_path).expect("test database should open"),
            map_config: MapConfig {
                provider: "amap",
                amap_web_js_api_key: None,
                amap_web_js_security_code: None,
                amap_android_key: None,
                amap_ios_key: None,
            },
        }
    }

    #[test]
    fn admin_url_keeps_query_on_admin_path() {
        let state = test_state("/adminfm6190123onwf");

        assert_eq!(
            admin_url(&state, "?message=auto%20cleanup%20saved"),
            "/adminfm6190123onwf?message=auto%20cleanup%20saved"
        );
    }

    #[test]
    fn admin_url_keeps_child_paths_under_admin_path() {
        let state = test_state("/adminfm6190123onwf");

        assert_eq!(admin_url(&state, "/login"), "/adminfm6190123onwf/login");
        assert_eq!(admin_url(&state, "login"), "/adminfm6190123onwf/login");
    }

    #[test]
    fn admin_notice_url_keeps_language_and_count() {
        let state = test_state("/adminfm6190123onwf");

        assert_eq!(
            admin_notice_url(&state, AdminLang::En, "stale_devices_deleted", Some(3)),
            "/adminfm6190123onwf?lang=en&msg=stale_devices_deleted&count=3"
        );
    }

    #[test]
    fn render_admin_page_uses_english_labels_and_keeps_language_in_forms() {
        let state = test_state("/adminfm6190123onwf");
        let html = render_admin_page(&state, &[], None, None, AdminLang::En);

        assert!(html.contains("<title>Guideng Admin</title>"));
        assert!(html.contains("Manual cleanup of inactive devices"));
        assert!(html.contains("Auto cleanup settings"));
        assert!(html.contains(r#"name="lang" value="en""#));
        assert!(html.contains(r#"href="/adminfm6190123onwf?lang=zh""#));
    }

    #[test]
    fn render_login_page_uses_english_labels() {
        let state = test_state("/adminfm6190123onwf");
        let html = render_login_page(&state, Some(AdminLang::En.login_error()), AdminLang::En);

        assert!(html.contains("Admin password"));
        assert!(html.contains("Incorrect password"));
        assert!(html.contains(r#"href="/adminfm6190123onwf/login?lang=zh""#));
        assert!(html.contains(r#"name="lang" value="en""#));
    }
}
