use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Path as AxumPath, Query, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use rand::{rngs::OsRng, Rng};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

const TOKEN_LEN: usize = 128;
const TOKEN_CHARS: &[u8] =
    b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ!@#$%^&*()-_=+[]{}:,.?/";

#[derive(Clone)]
struct AppState {
    token: Arc<String>,
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

#[tokio::main]
async fn main() -> Result<()> {
    let _log_guard = setup_logging()?;

    let token = load_or_generate_token();
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

    let store = Store::open(PathBuf::from(database_url))?;
    let state = AppState {
        token: Arc::new(token),
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

    let app = Router::new()
        .route("/health", get(health))
        .nest("/api", api)
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

fn generate_token() -> String {
    let mut rng = OsRng;
    (0..TOKEN_LEN)
        .map(|_| {
            let index = rng.gen_range(0..TOKEN_CHARS.len());
            TOKEN_CHARS[index] as char
        })
        .collect()
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
