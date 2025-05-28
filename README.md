# esbuild_rs

A Rust implementation of a client for communicating with esbuild's service API
over stdio. This project implements the binary protocol used by esbuild to
encode and decode messages between the client and the service.

## Usage

Create an `EsbuildService`, and send a build request:

```rust
use esbuild_rs::{EsbuildFlagsBuilder, EsbuildService, protocol::BuildRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // using a no-op plugin handler
    let esbuild = esbuild_rs::EsbuildService::new("/path/to/esbuild/binary", "0.25.5", None);
    let flags = esbuild_rs::EsbuildFlagsBuilder::default()
        .bundle(true)
        .minify(true)
        .format(esbuild::Format::Esm)
        .build()?
        .to_flags();

    let response = esbuild
        .client()
        .send_build_request(esbuild_rs::protocol::BuildRequest {
            entries: vec![("output.js".into(), "input.js".into())],
            flags,
            ..Default::default()
        })
        .await?;
    println!("build response: {:?}", response);

    Ok(())
}
```

Custom plugin handling:

```rust
use esbuild_rs::{EsbuildFlagsBuilder, EsbuildService, PluginHandler, protocol::BuildRequest};

struct MyPluginHandler;
#[async_trait::async_trait(?Send)]
impl PluginHandler for MyPluginHandler {
    async fn on_resolve(&self, args: OnResolveArgs) -> Result<Option<OnResolveResult>, AnyError> {
        println!("on_resolve: {:?}", args);
        Ok(None)
    }
    async fn on_load(&self, args: OnLoadArgs) -> Result<Option<OnLoadResult>, AnyError> {
        println!("on_load: {:?}", args);
        Ok(None)
    }
    async fn on_start(&self, args: OnStartArgs) -> Result<Option<OnStartResult>, AnyError> {
        println!("on_start: {:?}", args);
        Ok(None)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let esbuild = esbuild_rs::EsbuildService::new(
        "/path/to/esbuild/binary",
        "0.25.5",
        Arc::new(MyPluginHandler),
    );
    let flags = esbuild_rs::EsbuildFlagsBuilder::default()
        .bundle(true)
        .minify(true)
        .format(esbuild::Format::Esm)
        .build()?
        .to_flags();

    let response = esbuild
        .client()
        .send_build_request(esbuild_rs::protocol::BuildRequest {
            entries: vec![("output.js".into(), "input.js".into())],
            flags,
            plugins: Some(vec![esbuild_rs::protocol::BuildPlugin {
                name: "my-plugin".into(),
                on_resolve: vec![esbuild_rs::protocol::OnResolveSetupOptions {
                    id: 0,
                    filter: ".*".into(),
                    namespace: "my-plugin".into(),
                }],
                on_load: vec![esbuild_rs::protocol::OnLoadSetupOptions {
                    id: 0,
                    filter: ".*".into(),
                    namespace: "my-plugin".into(),
                }],
                on_start: true,
                on_end: false,
            }]),
            ..Default::default()
        })
        .await?;
    println!("build response: {:?}", response);

    Ok(())
}

```

## Layout

### Protocol Module (`src/protocol.rs`)

Defines the data structures and types that correspond to esbuild API messages:

- Structs representing various esbuild requests and responses (`BuildRequest`,
  `ServeRequest`, etc.)
- Message serialization/deserialization logic

### Encoding Module (`src/protocol/encode.rs`)

Implements the custom binary protocol encoding, based off of the implementation
in `npm:esbuild`.

### EsbuildService (`src/lib.rs`)

Manages the connection to the esbuild service:

- Spawns and manages the esbuild service process
- Handles communication via stdin/stdout
- Processes messages received from the service
