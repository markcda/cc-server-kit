# CC Server Kit

State-of-art simple and powerful web server based on `salvo`. Provides extended tracing, configuration-over-YAML, QUIC/HTTP3, MessagePack support, ACME, OpenAPI and OpenTelemetry features by default, with one step to CORS and WebSockets.

## How's it work

1. You load configuration from the file on the startup via `load_generic_config` function.
2. You start logging, check config for misconfigurations and load the state - all just via `load_generic_state` function.
3. You create your own `salvo::Router` and then generate server's `Future` and handle by `start` function.
4. You manually start awaiting `server`.

## 4 Quick start steps

1. Create `Setup` struct.
2. Create simple endpoints.
3. Create `server-example.yaml` file in crate root.
4. Just setup your application in 5 lines in `main`.

YAML configuration example:

```yaml
startup_type: http_localhost
server_port: 8801
allow_oapi_access: true
oapi_frontend_type: Scalar
oapi_name: Server Test OAPI
oapi_ver: 0.0.1
oapi_api_addr: /api
log_level: debug
```

`Cargo.toml`:

```toml
[package]
name = "cc-server-example"
version = "0.1.0"
edition = "2021"

[dependencies]
cc-server-kit = { git = "https://github.com/markcda/cc-server-kit.git", default-features = false, features = ["oapi", "utils"] }
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["macros"] }
```

The code itself:

```rust
use cc_server_kit::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, Default, Clone)]
struct Setup {
  #[serde(flatten)]
  generic_values: GenericValues,
  // this could be your global variables, such as the database URLs
}

impl GenericSetup for Setup {
  fn generic_values(&self) -> &GenericValues { &self.generic_values }
  fn generic_values_mut(&mut self) -> &mut GenericValues { &mut self.generic_values; }
}

#[derive(Deserialize, Serialize, Debug, salvo::oapi::ToSchema)]
/// Some hello
struct HelloData {
  /// Hello's text
  text: String,
}

#[endpoint(
  tags("test"),
  request_body(content = HelloData, content_type = "application/json", description = "Some JSON hello to MsgPack"),
  responses((status_code = 200, description = "Some MsgPack hello", body = HelloData, content_type = ["application/msgpack"]))
)]
#[instrument(skip_all, fields(http.uri = req.uri().path(), http.method = req.method().as_str()))]
async fn json_to_msgpack(req: &mut Request, depot: &mut Depot) -> MResult<MsgPack<HelloData>> {
  let hello = req.parse_json::<HelloData>().await?;
  let app_name = depot.obtain::<Setup>()?.generic_values().app_name.as_str();
  msgpack!(HelloData { text: format!("From `{}` application: {}", app_name, hello.text) })
}

#[endpoint(
  tags("test"),
  request_body(content = HelloData, content_type = "application/msgpack", description = "Some MsgPack hello to JSON"),
  responses((status_code = 200, description = "Some JSON hello", body = HelloData, content_type = ["application/json"]))
)]
#[instrument(skip_all, fields(http.uri = req.uri().path(), http.method = req.method().as_str()))]
async fn msgpack_to_json(req: &mut Request, depot: &mut Depot) -> MResult<Json<HelloData>> {
  let hello = req.parse_msgpack::<HelloData>().await?;
  let app_name = depot.obtain::<Setup>()?.generic_values().app_name.as_str();
  json!(HelloData { text: format!("From `{}` application: {}", app_name, hello.text) })
}

fn tests_router() -> Router {
  Router::new()
    .push(Router::with_path("msgpack-to-json").post(msgpack_to_json))
    .push(Router::with_path("json-to-msgpack").post(json_to_msgpack))
}

#[tokio::main]
async fn main() {
  let setup = load_generic_config::<Setup>("server-example").await.unwrap();
  let state = load_generic_state(&setup).await.unwrap();
  let router = get_root_router_autoinject(&state, setup.clone()).push(tests_router());
  let (server, _handler) = start(state, &setup, router).await.unwrap();
  server.await
}
```

Here we go! You can now start the server with `cargo run --release`!

## Configuring your server

### Startup type

You can select the startup type from this types:

1. `http_localhost` - will listen `http://127.0.0.1:{port}` only
2. `unsafe_http` - will listen `http://0.0.0.0:{port}`
3. `https_acme` (requires `acme` feature) - will listen `https://{host}:{port}` with [ACME] support
4. `quinn_acme` (requires both `acme` and `http3` features) - will listen `https://` and `quic://` with [ACME]
5. `https_only` - will listen `https://{host}:{port}`
6. `quinn` (requires `http3` feature) - will listen `https://` and `quic://`
7. `quinn_only` (requires `http3` feature) - will listen `quic://{host}:{port}`

Example:

```yaml
startup_type: quinn
```

### Server host & server port

Specify `server_host` as IP address to listen with server (except `http_localhost` and `unsafe_http` startup types).

Specify `server_port` to listen with server. If you use your app with CC Server Kit as internal service, specify any port; if you want to expose your ports to the Internet, use `80` to HTTP and `443` for HTTPS or QUIC.

Also, if you want to specify your listening port after application start, you can use `server_port_achiever` field (see below).

Example:

```yaml
startup_type: quinn
server_host: 0.0.0.0
server_port: 443
```

### ACME domain

Specify `acme_domain` to use [ACME] (TLS ALPN-01).

Example:

```yaml
startup_type: quinn_acme
server_host: 0.0.0.0
server_port: 443
acme_domain: tls-alpn-01.domain.com
```

### SSL key & certs

Example:

```yaml
startup_type: quinn
server_host: 0.0.0.0
server_port: 443
ssl_crt_path: certs/fullchain.pem
ssl_key_path: certs/privkey.pem
```

### Auto-migrate binary

Specify `auto_migrate_bin` field to automatically execute any binary (for example, DB migrations) before actual server start.

### Allow CORS

Specify `allow_cors_domain` field to automatically manage CORS policy to given domain or domains.

Example:

```yaml
# ...
allow_cors_domain: "https://my-domain.com"
```

### Allow OAPI

Specify `allow_oapi_access` field to automatically generate OpenAPI specifications and provide to users.

Example:

```yaml
# ...
allow_oapi_access: true
oapi_frontend_type: Scalar # or `SwaggerUI`
oapi_name: My API
oapi_ver: 0.1.0
```

### Logging

CC Server Kit uses `tracing` for logging inside routes' logic. Configuration example:

```yaml
log_level: info       # error | warn | info | debug | trace
log_file_level: debug # error | warn | info | debug | trace
log_rolling: daily    # never | daily | hourly | minutely
log_rolling_max_files: 5
```

You can also specify `open_telemetry_endpoint` to automatically send your metrics collected with `tracing` to anything like Prometheus or Jaeger.

### Server port achieveing

You can specify `server_port_achiever` field to any filepath to make server wait for file creation and writing actual server port to listen to it.

Example:

```yaml
startup_type: quinn
server_host: 0.0.0.0
server_port_achiever: write/port/to/me.txt
```

### Force HTTPS

To enforce HTTPS, you should start another server via `start_force_https_redirect` function:

```rust
let (server, handler) = start_force_https_redirect(80, 443).await.unwrap();
```

[ACME]: https://en.wikipedia.org/wiki/Automatic_Certificate_Management_Environment
