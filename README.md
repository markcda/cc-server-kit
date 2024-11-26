# CC Server Kit

State-of-art simple and powerful web server based on `salvo`. Provides extended tracing, configuration-over-YAML, QUIC/HTTP3, MessagePack support, ACME, OpenAPI and OpenTelemetry features by default, with one step to CORS and WebSockets.

## 4 Quick start steps

1. Create `Setup` struct.
2. Create simple endpoints.
3. Create `server-example.yaml` file in crate root.
4. Just setup your application in 7 lines in `main`.

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

#[derive(Default, Clone)]
struct Setup {
  generic_values: GenericValues,
  // this could be your global variables, such as the database URLs
}

impl GenericSetup for Setup {
  fn generic_values(&self) -> &GenericValues { &self.generic_values }
  fn set_generic_values(&mut self, generic_values: GenericValues) { self.generic_values = generic_values; }
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
  let router = get_root_router(&state, setup.clone()).push(tests_router());
  let (server, _handler) = start(state, &setup, router).await.unwrap();
  server.await
}
```

Here we go! You can now start the server with `cargo run --release`!
