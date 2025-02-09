//! Setup module.

use salvo::http::StatusCode;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_appender::non_blocking::WorkerGuard as TracingFileGuard;

use cc_utils::prelude::*;

static E500: StatusCode = StatusCode::INTERNAL_SERVER_ERROR;

/// Provides at least values needed by Server Kit to start.
pub trait GenericSetup {
  /// Provides generic values; see `GenericValues`.
  fn generic_values(&self) -> &GenericValues;
  /// Provides mutable generic values; see `GenericValues`.
  fn generic_values_mut(&mut self) -> &mut GenericValues;
}

/// Server startup variants.
///
/// These are the hardcoded variants; by default, `salvo` can much more than this.
#[derive(Clone, Eq, PartialEq)]
pub enum StartupVariant {
  /// Will listen `http://127.0.0.1:{port}` only.
  HttpLocalhost,
  /// Will listen `http://{host}:{port}`. Not recommended.
  UnsafeHttp,
  #[cfg(feature = "acme")]
  /// Will listen `https://{host}:{port}` with automatic SSL certificate acquiring.
  HttpsAcme,
  #[cfg(all(feature = "http3", feature = "acme"))]
  /// Will listen `https|quic://{host}:{port}` with automatic SSL certificate acquiring.
  QuinnAcme,
  /// Will listen `https://{host}:{port}` with your SSL cert and key.
  HttpsOnly,
  #[cfg(feature = "http3")]
  /// Will listen `https|quic://{host}:{port}` with your SSL cert and key.
  Quinn,
  #[cfg(feature = "http3")]
  /// Will listen `quic://{host}:{port}` only, with your SSL cert and key.
  QuinnOnly,
}

/// Server generic configuration.
#[derive(Clone, Deserialize)]
pub struct GenericValues {
  /// Application name.
  ///
  /// You're not needed to write it in YAML configuration, instead you should send it to `load_generic_config` function.
  #[serde(skip)]
  pub app_name: String,
  /// Startup variant. Converts to `StartupVariant`.
  pub startup_type: String,
  /// Server host.
  pub server_host: Option<String>,
  /// Server port. For no reverse proxy and Internet usage, set to `80` for HTTP and `443` for HTTPS/QUIC.
  pub server_port: Option<u16>,
  /// ACME origin; see [`salvo/conn/acme` docs](https://docs.rs/salvo/latest/salvo/conn/acme/index.html).
  pub acme_domain: Option<String>,
  /// Path to SSL key.
  pub ssl_key_path: Option<String>,
  /// Path to SSL certificate.
  pub ssl_crt_path: Option<String>,
  /// If you want to run any migration or anything else just before server's start, set to path to binary.
  pub auto_migrate_bin: Option<String>,
  /// Use text file to find out which port to listen to.
  pub server_port_achiever: Option<PathBuf>,

  #[cfg(feature = "cors")]
  /// CORS allowed domains
  pub allow_cors_domain: Option<String>,

  #[cfg(feature = "oapi")]
  /// Set this to `true` to enable OpenAPI endpoint.
  pub allow_oapi_access: Option<bool>,
  #[cfg(feature = "oapi")]
  /// Select `Scalar` or `SwaggerUI`.
  pub oapi_frontend_type: Option<String>,
  #[cfg(feature = "oapi")]
  /// By default, equals `app_name`; consider give expanded API name.
  pub oapi_name: Option<String>,
  #[cfg(feature = "oapi")]
  /// API version.
  pub oapi_ver: Option<String>,
  #[cfg(feature = "oapi")]
  /// API endpoint (with slash), e.g. `/api` or `/swagger`.
  pub oapi_api_addr: Option<String>,

  /// Log level; for no logging delete the line in YAML completely.
  pub log_level: Option<String>,
  /// File's log level; for no logging delete the line in YAML completely.
  pub log_file_level: Option<String>,
  /// File rolling, if you have a ton of logs and need to split them.
  pub log_rolling: Option<String>,
  /// Files limitation for autoremove.
  pub log_rolling_max_files: Option<u32>,

  #[cfg(feature = "otel")]
  /// Endpoint to export OpenTelemetry (e.g., Jaeger).
  pub open_telemetry_endpoint: Option<String>,
}

impl Default for GenericValues {
  fn default() -> Self {
    Self {
      app_name: "generic".into(),
      startup_type: "http_localhost".into(),
      server_host: None,
      server_port: Some(8800),
      acme_domain: None,
      ssl_key_path: None,
      ssl_crt_path: None,
      auto_migrate_bin: None,
      #[cfg(feature = "cors")]
      allow_cors_domain: None,
      #[cfg(feature = "oapi")]
      allow_oapi_access: None,
      #[cfg(feature = "oapi")]
      oapi_frontend_type: None,
      #[cfg(feature = "oapi")]
      oapi_name: None,
      #[cfg(feature = "oapi")]
      oapi_ver: None,
      #[cfg(feature = "oapi")]
      oapi_api_addr: None,
      log_level: Some("debug".into()),
      log_file_level: None,
      log_rolling: None,
      log_rolling_max_files: None,
      #[cfg(feature = "otel")]
      open_telemetry_endpoint: None,
      server_port_achiever: None,
    }
  }
}

/// Server state.
#[derive(Clone)]
pub struct GenericServerState {
  /// Converted startup variant, ready to launch.
  pub startup_variant: StartupVariant,
  /// File log guard; needed to be handled the entire time the application is running.
  pub _file_log_guard: Option<Arc<TracingFileGuard>>,
}

async fn watcher<P: AsRef<std::path::Path>>(path: P) -> MResult<u16> {
  use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

  let (tx, mut rx) = tokio::sync::mpsc::channel(1);
  let mut watcher = RecommendedWatcher::new(move |res| tx.blocking_send(res).unwrap(), Config::default())
    .map_err(|e| ErrorResponse::from(e.to_string()).with_500_pub().build())?;
  watcher
    .watch(path.as_ref(), RecursiveMode::NonRecursive)
    .map_err(|e| ErrorResponse::from(e.to_string()).with_500_pub().build())?;

  while let Some(res) = rx.recv().await {
    match res {
      Ok(event) if event.kind.is_modify() || event.kind.is_create() => {
        if let Ok(port) = std::fs::read_to_string(path.as_ref())
          && let Ok(port) = port.trim().parse::<u16>()
        {
          watcher
            .unwatch(path.as_ref())
            .map_err(|e| ErrorResponse::from(e.to_string()).with_500_pub().build())?;
          return Ok(port);
        }
      }
      Err(e) => {
        tracing::error!("Watch error: {:?}", e);
        return Err(ErrorResponse::from(e.to_string()).with_500_pub().build());
      }
      _ => {}
    }
  }

  Err(ErrorResponse::from("Event channel is broken!").with_500_pub().build())
}

/// Loads the config from YAML file (`{app_name}.yaml`).
pub async fn load_generic_config<T: DeserializeOwned + GenericSetup + Default>(app_name: &str) -> MResult<T> {
  let mut file = std::fs::File::open(format!("{}.yaml", app_name));
  if file.is_err() {
    file = std::fs::File::open(format!("/etc/{}.yaml", app_name));
  }
  let mut file = file.consider(Some(E500), Some("The server configuration could not be found."), true)?;

  let mut buffer = String::new();
  file.read_to_string(&mut buffer).consider(
    Some(E500),
    Some("Failed to read the contents of the server configuration file."),
    true,
  )?;
  let mut config: T = serde_yaml::from_str(&buffer).map_err(|_| {
    ErrorResponse::from("Failed to parse the contents of the server configuration file.")
      .with_500_pub()
      .build()
  })?;

  let data = config.generic_values_mut();
  data.app_name = app_name.to_string();

  #[cfg(feature = "oapi")]
  if data.allow_oapi_access.is_some_and(|v| v) {
    if data.oapi_name.is_none() {
      return Err(
        ErrorResponse::from("The API name for OAPI is not specified.")
          .with_500_pub()
          .build(),
      );
    }
    if data.oapi_ver.is_none() {
      return Err(
        ErrorResponse::from("The API version for OAPI is not specified.")
          .with_500_pub()
          .build(),
      );
    }
    if data.oapi_api_addr.is_none() {
      return Err(
        ErrorResponse::from("The path to OAPI was not specified.")
          .with_500_pub()
          .build(),
      );
    }
  }

  if let Some(achiever) = &data.server_port_achiever {
    let port = watcher(achiever.as_path()).await?;
    data.server_port = Some(port);
  }

  Ok(config)
}

/// Loads the server's state: initializes the logging and checks YAML config for misconfigurations and errors.
pub async fn load_generic_state<T: GenericSetup>(setup: &T) -> MResult<GenericServerState> {
  let data = setup.generic_values();

  let log_level = match_log_level(&data.log_level);
  let log_file_level = match_log_level(&data.log_file_level);
  let log_rolling = match_log_file_rolling(&data.log_rolling)?;

  let file_log_guard = init_logging(
    &setup.generic_values().app_name,
    &log_level,
    &log_file_level,
    log_rolling,
    &data.log_rolling_max_files,
    #[cfg(feature = "otel")]
    &data.open_telemetry_endpoint,
  )?;

  let state = GenericServerState {
    startup_variant: match &*data.startup_type {
      "http_localhost" => {
        if data.server_host.is_some() { return Err(ErrorResponse::from("Server will only listen `127.0.0.1` address because of `http_localhost` startup variant. Consider to move to `https_only` or `quinn`.").with_500_pub().build()) }
        StartupVariant::HttpLocalhost
      },
      "unsafe_http" => {
        if data.server_host.is_none() { return Err(ErrorResponse::from("Choose server's host, e.g. `0.0.0.0`.").with_500_pub().build()) }
        StartupVariant::UnsafeHttp
      }
      #[cfg(feature = "acme")]
      "https_acme" => {
        if data.server_host.is_none() { return Err(ErrorResponse::from("Choose server's host, e.g. `0.0.0.0`.").with_500_pub().build()) }
        if data.acme_domain.is_none() { return Err(ErrorResponse::from("Choose ACME's domain!").with_500_pub().build()) }
        StartupVariant::HttpsAcme
      },
      "https_only" => {
        if data.server_host.is_none() { return Err(ErrorResponse::from("Choose server's host, e.g. `0.0.0.0`.").with_500_pub().build()) }
        if data.ssl_key_path.is_none() { return Err(ErrorResponse::from("Choose SSL key path.").with_500_pub().build()) }
        if data.ssl_crt_path.is_none() { return Err(ErrorResponse::from("Choose SSL cert path.").with_500_pub().build()) }
        StartupVariant::HttpsOnly
      },
      #[cfg(all(feature = "http3", feature = "acme"))]
      "quinn_acme" => {
        if data.server_host.is_none() { return Err(ErrorResponse::from("Choose server's host, e.g. `0.0.0.0`.").with_500_pub().build()) }
        if data.acme_domain.is_none() { return Err(ErrorResponse::from("Choose ACME's domain!").with_500_pub().build()) }
        StartupVariant::QuinnAcme
      },
      #[cfg(feature = "http3")]
      "quinn" => {
        if data.server_host.is_none() { return Err(ErrorResponse::from("Choose server's host, e.g. `0.0.0.0`.").with_500_pub().build()) }
        if data.ssl_key_path.is_none() { return Err(ErrorResponse::from("Choose SSL key path.").with_500_pub().build()) }
        if data.ssl_crt_path.is_none() { return Err(ErrorResponse::from("Choose SSL cert path.").with_500_pub().build()) }
        StartupVariant::Quinn
      },
      #[cfg(feature = "http3")]
      "quinn_only" => {
        if data.server_host.is_none() { return Err(ErrorResponse::from("Choose server's host, e.g. `0.0.0.0`.").with_500_pub().build()) }
        if data.ssl_key_path.is_none() { return Err(ErrorResponse::from("Choose SSL key path.").with_500_pub().build()) }
        if data.ssl_crt_path.is_none() { return Err(ErrorResponse::from("Choose SSL cert path.").with_500_pub().build()) }
        StartupVariant::QuinnOnly
      },
      _ => return Err(ErrorResponse::from("The server deployment method could not be determined. Read the documentation on the `startup_variant` field.").with_500_pub().build()),
    },
    _file_log_guard: file_log_guard.map(Arc::new),
  };
  Ok(state)
}

fn match_log_level(log_level: &Option<String>) -> MResult<tracing::Level> {
  if log_level.is_some() {
    Ok(match log_level.as_ref().unwrap().as_str() {
      "error" => tracing::Level::ERROR,
      "warn" => tracing::Level::WARN,
      "info" => tracing::Level::INFO,
      "debug" => tracing::Level::DEBUG,
      "trace" => tracing::Level::TRACE,
      _ => return Err(ErrorResponse::from("Incorrect logging level.").with_500_pub().build()),
    })
  } else if cfg!(debug_assertions) {
    Ok(tracing::Level::DEBUG)
  } else {
    Err(ErrorResponse::from("Logging is disabled").with_500_pub().build())
  }
}

fn match_log_file_rolling(log_rolling: &Option<String>) -> MResult<tracing_appender::rolling::Rotation> {
  if let Some(log_rolling) = log_rolling {
    Ok(match log_rolling.as_str() {
      "never" => tracing_appender::rolling::Rotation::NEVER,
      "daily" => tracing_appender::rolling::Rotation::DAILY,
      "hourly" => tracing_appender::rolling::Rotation::HOURLY,
      "minutely" => tracing_appender::rolling::Rotation::MINUTELY,
      _ => {
        return Err(
          ErrorResponse::from(
            "Incorrect level of log rotation. Choose one of the options: `never`, `daily`, `hourly`, `minutely`.",
          )
          .with_500()
          .build(),
        );
      }
    })
  } else {
    Ok(tracing_appender::rolling::Rotation::NEVER)
  }
}

#[allow(dead_code)]
fn log_filter(metadata: &tracing::Metadata) -> bool {
  metadata.module_path().is_none_or(|p| {
    !(p.contains("salvo") || p.contains("hyper_util") || p.contains("tower") || p.contains("quinn") || p.contains("h2"))
  })
}

fn init_logging(
  app_name: &str,
  log_level: &MResult<tracing::Level>,
  log_file_level: &MResult<tracing::Level>,
  log_rolling: tracing_appender::rolling::Rotation,
  log_rolling_max_files: &Option<u32>,
  #[cfg(feature = "otel")] open_telemetry_endpoint: &Option<String>,
) -> MResult<Option<TracingFileGuard>> {
  use tracing_appender::rolling;
  #[allow(unused_imports)]
  use tracing_subscriber::filter::{LevelFilter, filter_fn};
  use tracing_subscriber::fmt::format::FmtSpan;
  use tracing_subscriber::prelude::*;
  use tracing_subscriber::{fmt, registry};

  #[cfg(feature = "otel")]
  use crate::otel::api::{KeyValue, trace::TracerProvider};
  #[cfg(feature = "otel")]
  use crate::otel::exporter::WithExportConfig;
  #[cfg(feature = "otel")]
  use crate::otel::sdk::{Resource, trace::RandomIdGenerator};

  let format = fmt::format()
    .with_level(true)
    .with_target(true)
    .with_thread_ids(false)
    .with_thread_names(false)
    .with_file(false)
    .with_line_number(true)
    .compact();

  let io_tracer = if let Ok(log_level) = log_level {
    #[cfg(not(feature = "log-without-filtering"))]
    let io_tracer = fmt::layer()
      .event_format(format.clone())
      .with_writer(std::io::stdout)
      .with_span_events(FmtSpan::CLOSE)
      .with_filter(LevelFilter::from_level(*log_level))
      .with_filter(filter_fn(log_filter));
    #[cfg(feature = "log-without-filtering")]
    let io_tracer = fmt::layer()
      .event_format(format.clone())
      .with_writer(std::io::stdout)
      .with_span_events(FmtSpan::CLOSE)
      .with_filter(LevelFilter::from_level(*log_level));
    Some(io_tracer)
  } else {
    None
  };

  let (file_tracer, guard) = if let Ok(log_file_level) = log_file_level {
    let file_appender = rolling::RollingFileAppender::builder()
      .rotation(log_rolling)
      .filename_suffix(app_name)
      .max_log_files(log_rolling_max_files.unwrap_or(5) as usize)
      .build("logs")
      .map_err(|_| {
        ErrorResponse::from("Failed to initialize logging to file!")
          .with_500_pub()
          .build()
      })?;
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    #[cfg(not(feature = "log-without-filtering"))]
    let file_tracer = fmt::layer()
      .event_format(format)
      .with_writer(non_blocking)
      .with_ansi(false)
      .with_span_events(FmtSpan::CLOSE)
      .with_filter(LevelFilter::from_level(*log_file_level))
      .with_filter(filter_fn(log_filter));
    #[cfg(feature = "log-without-filtering")]
    let file_tracer = fmt::layer()
      .event_format(format)
      .with_writer(non_blocking)
      .with_ansi(false)
      .with_span_events(FmtSpan::CLOSE)
      .with_filter(LevelFilter::from_level(*log_file_level));

    (Some(file_tracer), Some(guard))
  } else {
    (None, None)
  };

  #[cfg(feature = "otel")]
  let otel_tracer = if let Some(open_telemetry_endpoint) = open_telemetry_endpoint
    && let Ok(log_level) = log_level
  {
    let otel_span_exporter = opentelemetry_otlp::SpanExporter::builder()
      .with_tonic()
      .with_endpoint(open_telemetry_endpoint.as_str())
      .build()
      .map_err(|_| ErrorResponse::from("Failed to initialize OTEL telemetry!"))?;
    let otel_provider = opentelemetry_sdk::trace::TracerProvider::builder()
      .with_simple_exporter(otel_span_exporter)
      .with_id_generator(RandomIdGenerator::default())
      .with_max_events_per_span(32)
      .with_max_attributes_per_span(64)
      .with_resource(Resource::new(vec![KeyValue::new("service.name", app_name.to_owned())]))
      .build()
      .tracer(app_name.to_owned());

    #[cfg(not(feature = "log-without-filtering"))]
    let opentelemetry = tracing_opentelemetry::layer()
      .with_tracer(otel_provider)
      .with_filter(LevelFilter::from_level(*log_level))
      .with_filter(filter_fn(log_filter));
    #[cfg(feature = "log-without-filtering")]
    let opentelemetry = tracing_opentelemetry::layer()
      .with_tracer(otel_provider)
      .with_filter(LevelFilter::from_level(*log_level));

    Some(opentelemetry)
  } else {
    None
  };

  #[cfg(feature = "otel")]
  let collector = registry().with(io_tracer).with(file_tracer).with(otel_tracer);
  #[cfg(not(feature = "otel"))]
  let collector = registry().with(io_tracer).with(file_tracer);

  tracing::subscriber::set_global_default(collector)?;

  Ok(guard)
}
