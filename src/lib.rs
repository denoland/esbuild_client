use std::{
    collections::HashMap,
    fmt::Display,
    ops::Deref,
    path::Path,
    process::{ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

pub use anyhow::Error as AnyError;
use async_trait::async_trait;
use indexmap::IndexMap;
use parking_lot::Mutex;
use protocol::{
    AnyPacket, Encode, FromAnyValue, FromMap, ImportKind, OnStartResponse, PartialMessage,
    ProtocolPacket,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{ChildStdin, ChildStdout},
    sync::{mpsc, oneshot, watch},
};
pub mod protocol;

pub struct EsbuildService {
    exited: watch::Receiver<Option<ExitStatus>>,
    client: ProtocolClient,
}

impl EsbuildService {
    pub fn client(&self) -> &ProtocolClient {
        &self.client
    }

    pub async fn wait_for_exit(&self) -> Result<ExitStatus, AnyError> {
        if self.exited.borrow().is_some() {
            return Ok(self.exited.borrow().unwrap());
        }
        let mut exited = self.exited.clone();

        let _ = exited.changed().await;
        let status = exited.borrow().unwrap();
        Ok(status)
    }
}

// fn handle_packet(is_first_packet: bool, packet: &[u8]) {

// }
//

struct ProtocolState {
    buffer: Vec<u8>,
    read_into_offset: usize,
    offset: usize,
    first_packet: bool,
    ready_tx: Option<oneshot::Sender<()>>,
    packet_tx: mpsc::Sender<AnyPacket>,
}

impl ProtocolState {
    fn new(ready_tx: oneshot::Sender<()>, packet_tx: mpsc::Sender<AnyPacket>) -> Self {
        let mut buffer = Vec::with_capacity(1024);
        buffer.extend(std::iter::repeat(0).take(1024));
        let read_into_offset = 0;
        let offset = 0;

        let first_packet = true;
        Self {
            buffer,
            first_packet,
            offset,
            read_into_offset,
            ready_tx: Some(ready_tx),
            packet_tx,
        }
    }
}

async fn handle_read(amount: usize, state: &mut ProtocolState) {
    state.read_into_offset += amount;
    if state.read_into_offset >= state.buffer.len() {
        state.buffer.extend(std::iter::repeat(0).take(1024));
    }
    while state.offset + 4 <= state.read_into_offset {
        let length = u32::from_le_bytes(
            state.buffer[state.offset..state.offset + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        // eprintln!(
        //     "length: {}; offset: {}; read_into_offset: {}",
        //     length, offset, read_into_offset
        // );
        if state.offset + 4 + length > state.read_into_offset {
            break;
        }
        state.offset += 4;

        let message = &state.buffer[state.offset..state.offset + length];
        // eprintln!("here");
        if state.first_packet {
            // eprintln!("first packet");
            state.first_packet = false;
            // let version = String::from_utf8(message.to_vec()).unwrap();
            // eprintln!("version: {}", version);
            state.ready_tx.take().unwrap().send(()).unwrap();
        } else {
            match protocol::decode_any_packet(message) {
                Ok(packet) => {
                    // eprintln!("decoded packet: {packet:?}");
                    state.packet_tx.send(packet).await.unwrap()
                }
                Err(e) => eprintln!("Error decoding packet: {}", e),
            }
        }

        state.offset += length;
    }
}

async fn protocol_task(
    stdout: ChildStdout,
    stdin: ChildStdin,
    ready_tx: oneshot::Sender<()>,
    mut response_rx: mpsc::Receiver<ProtocolPacket>,
    packet_tx: mpsc::Sender<AnyPacket>,
) -> Result<(), AnyError> {
    let mut stdout = stdout;

    let mut state = ProtocolState::new(ready_tx, packet_tx);
    let mut stdin = stdin;

    loop {
        tokio::select! {
            res = response_rx.recv() => {
                let packet: protocol::ProtocolPacket = res.unwrap();
                // eprintln!("got send packet from receiver: {packet:?}");
                let mut encoded = Vec::new();
                packet.encode_into(&mut encoded);
                // eprintln!("encoded: {:?}", encoded);
                stdin.write_all(&encoded).await?;
                // eprintln!("wrote packet");
            }
            read_length = stdout.read(&mut state.buffer[state.read_into_offset..]) => {
                let Ok(read_length) = read_length else {
                    eprintln!("Error reading stdout");
                    continue;
                };
                handle_read(read_length, &mut state).await;
                // eprintln!(
                //     "read_length: {}; read_into_offset: {}; offset: {}; buffer.len(): {}",
                //     read_length,
                //     read_into_offset,
                //     offset,
                //     buffer.len()
                //     );

            }
        }
    }

    #[allow(unreachable_code)]
    Ok::<(), AnyError>(())
}

pub trait MakePluginHandler {
    fn make_plugin_handler(self, client: ProtocolClient) -> Arc<dyn PluginHandler>;
}

impl<F> MakePluginHandler for F
where
    F: FnOnce(ProtocolClient) -> Arc<dyn PluginHandler>,
{
    fn make_plugin_handler(self, client: ProtocolClient) -> Arc<dyn PluginHandler> {
        (self)(client)
    }
}

impl MakePluginHandler for Arc<dyn PluginHandler> {
    fn make_plugin_handler(self, _client: ProtocolClient) -> Arc<dyn PluginHandler> {
        self
    }
}

impl<T: PluginHandler + 'static> MakePluginHandler for Arc<T> {
    fn make_plugin_handler(self, _client: ProtocolClient) -> Arc<dyn PluginHandler> {
        self
    }
}
impl EsbuildService {
    pub async fn new(
        path: impl AsRef<Path>,
        version: &str,
        plugin_handler: impl MakePluginHandler,
    ) -> Result<Self, AnyError> {
        let path = path.as_ref();
        let mut esbuild = tokio::process::Command::new(path)
            .arg(&format!("--service={}", version))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let stdin = esbuild.stdin.take().unwrap();
        let stdout = esbuild.stdout.take().unwrap();

        let (ready_tx, ready_rx) = oneshot::channel();
        let (packet_tx, mut packet_rx) = mpsc::channel(100);
        let (response_tx, response_rx) = mpsc::channel(100);
        let (exited_tx, exited_rx) = watch::channel(None);
        tokio::spawn(protocol_task(
            stdout,
            stdin,
            ready_tx,
            response_rx,
            packet_tx,
        ));

        tokio::spawn(async move {
            let status = esbuild.wait().await.unwrap();
            let _ = exited_tx.send(Some(status));
        });

        let client = ProtocolClient::new(response_tx.clone());
        let plugin_handler = plugin_handler.make_plugin_handler(client.clone());
        let pending = client.0.pending.clone();

        tokio::spawn(async move {
            loop {
                let packet = packet_rx.recv().await;
                // eprintln!("got packet from receiver: {packet:?}");

                if let Some(packet) = packet {
                    let _ = handle_packet(packet, &response_tx, plugin_handler.clone(), &pending)
                        .await
                        .inspect_err(|err| {
                            eprintln!("failed to handle packet {err}");
                        });
                }
            }
        });

        let _ = ready_rx.await;

        Ok(Self {
            exited: exited_rx,
            client,
        })
    }
}

#[derive(Debug, Clone)]
pub struct OnResolveArgs {
    pub key: u32,
    pub ids: Vec<u32>,
    pub path: String,
    pub importer: Option<String>,
    pub kind: ImportKind,
    pub namespace: Option<String>,
    pub resolve_dir: Option<String>,
    pub with: IndexMap<String, String>,
}

#[derive(Debug, Default)]
pub struct OnResolveResult {
    pub plugin_name: Option<String>,
    pub errors: Option<Vec<protocol::PartialMessage>>,
    pub warnings: Option<Vec<protocol::PartialMessage>>,
    pub path: Option<String>,
    pub external: Option<bool>,
    pub side_effects: Option<bool>,
    pub namespace: Option<String>,
    pub suffix: Option<String>,
    pub plugin_data: Option<u32>,
    pub watch_files: Option<Vec<String>>,
    pub watch_dirs: Option<Vec<String>>,
}

// pub trait PluginInfo {
//     fn name(&self) -> &'static str;
//     fn provides_on_resolve(&self) -> bool {
//         false
//     }
// }

// pub trait Plugin: PluginInfo {
//     fn on_resolve(&self, _args: OnResolveArgs) -> Result<Option<OnResolveResult>, AnyError> {
//         anyhow::bail!("not implemented")
//     }
//     // fn on_load(&self)
// }

#[async_trait]
pub trait PluginHandler: Send + Sync {
    async fn on_resolve(&self, _args: OnResolveArgs) -> Result<Option<OnResolveResult>, AnyError>;
    async fn on_load(&self, _args: OnLoadArgs) -> Result<Option<OnLoadResult>, AnyError>;
}

pub struct OnLoadResult {
    pub id: Option<u32>,
    pub plugin_name: Option<String>,
    pub errors: Option<Vec<PartialMessage>>,
    pub warnings: Option<Vec<PartialMessage>>,
    pub contents: Option<Vec<u8>>,
    pub resolve_dir: Option<String>,
    pub loader: Option<BuiltinLoader>,
    pub plugin_data: Option<u32>,
    pub watch_files: Option<Vec<String>>,
    pub watch_dirs: Option<Vec<String>>,
}

#[derive(Debug)]
pub struct OnLoadArgs {
    pub key: u32,
    pub ids: Vec<u32>,
    pub path: String,
    pub namespace: String,
    pub suffix: String,
    pub plugin_data: Option<u32>,
    pub with: IndexMap<String, String>,
}

async fn handle_packet(
    packet: AnyPacket,
    response_tx: &mpsc::Sender<protocol::ProtocolPacket>,
    plugin_handler: Arc<dyn PluginHandler>,
    pending: &Mutex<PendingResponseMap>,
) -> Result<(), AnyError> {
    match &packet.value {
        protocol::AnyValue::Map(index_map) => {
            if packet.is_request {
                match index_map.get("command").map(|v| v.as_string()) {
                    Some(Ok(s)) => match s.as_str() {
                        "on-start" => {
                            let _on_start = protocol::OnStartRequest::from_map(index_map)?;
                            eprintln!("on-start: {:?}", _on_start);
                            response_tx
                                .send(protocol::ProtocolPacket {
                                    id: packet.id,
                                    is_request: false,
                                    value: protocol::ProtocolMessage::Response(
                                        protocol::AnyResponse::OnStart(OnStartResponse {
                                            errors: vec![],
                                            warnings: vec![],
                                        }),
                                    ),
                                })
                                .await?;
                            Ok(())
                        }
                        "on-resolve" => {
                            // eprintln!("got on resolve");
                            let on_resolve = protocol::OnResolveRequest::from_map(index_map)?;

                            fn empty_none(s: String) -> Option<String> {
                                if s.is_empty() { None } else { Some(s) }
                            }

                            let id = packet.id;
                            // eprintln!("on-resolve: {:?}", on_resolve);
                            let response_tx = response_tx.clone();
                            tokio::spawn(async move {
                                let result = plugin_handler
                                    .on_resolve(OnResolveArgs {
                                        key: on_resolve.key,
                                        ids: on_resolve.ids,
                                        path: on_resolve.path,
                                        importer: empty_none(on_resolve.importer),
                                        kind: on_resolve.kind,
                                        namespace: empty_none(on_resolve.namespace),
                                        resolve_dir: on_resolve.resolve_dir,
                                        with: on_resolve.with,
                                    })
                                    .await;
                                match result {
                                    Ok(Some(on_resolve_result)) => {
                                        // eprintln!("on-resolve result: {:?}", on_resolve_result);
                                        let response = protocol::OnResolveResponse {
                                            id: Some(id),
                                            plugin_name: on_resolve_result.plugin_name,
                                            errors: None,
                                            warnings: None,
                                            path: on_resolve_result.path,
                                            external: on_resolve_result.external,
                                            side_effects: on_resolve_result.side_effects,
                                            namespace: on_resolve_result.namespace,
                                            suffix: on_resolve_result.suffix,
                                            plugin_data: on_resolve_result.plugin_data,
                                            watch_files: on_resolve_result.watch_files,
                                            watch_dirs: on_resolve_result.watch_dirs,
                                        };
                                        let _ = response_tx
                                            .send(protocol::ProtocolPacket {
                                                id,
                                                is_request: false,
                                                value: protocol::ProtocolMessage::Response(
                                                    protocol::AnyResponse::OnResolve(response),
                                                ),
                                            })
                                            .await;
                                    }
                                    Ok(None) => {
                                        let response = protocol::OnResolveResponse {
                                            id: Some(id),
                                            plugin_name: None,
                                            errors: None,
                                            warnings: None,
                                            path: None,
                                            external: None,
                                            side_effects: None,
                                            namespace: None,
                                            suffix: None,
                                            plugin_data: None,
                                            watch_files: None,
                                            watch_dirs: None,
                                        };
                                        let _ = response_tx
                                            .send(protocol::ProtocolPacket {
                                                id,
                                                is_request: false,
                                                value: protocol::ProtocolMessage::Response(
                                                    protocol::AnyResponse::OnResolve(response),
                                                ),
                                            })
                                            .await;
                                    }
                                    Err(e) => eprintln!("Error on-resolve: {}", e),
                                }
                            });

                            return Ok(());
                        }
                        "on-load" => {
                            let on_load = protocol::OnLoadRequest::from_map(index_map)?;
                            let response_tx = response_tx.clone();

                            let id = packet.id;
                            tokio::spawn(async move {
                                let result = plugin_handler
                                    .on_load(OnLoadArgs {
                                        key: on_load.key,
                                        path: on_load.path,
                                        ids: on_load.ids,
                                        namespace: on_load.namespace,
                                        suffix: on_load.suffix,
                                        plugin_data: on_load.plugin_data,
                                        with: on_load.with,
                                    })
                                    .await;
                                match result {
                                    Ok(Some(on_load_result)) => {
                                        let response = protocol::OnLoadResponse {
                                            id: on_load_result.id,
                                            plugin_name: on_load_result.plugin_name,
                                            errors: on_load_result.errors,
                                            warnings: on_load_result.warnings,
                                            contents: on_load_result.contents,
                                            resolve_dir: on_load_result.resolve_dir,
                                            loader: on_load_result
                                                .loader
                                                .map(|loader| loader.to_string()),
                                            plugin_data: on_load_result.plugin_data,
                                            watch_files: on_load_result.watch_files,
                                            watch_dirs: on_load_result.watch_dirs,
                                        };
                                        let _ = response_tx
                                            .send(protocol::ProtocolPacket {
                                                id,
                                                is_request: false,
                                                value: protocol::ProtocolMessage::Response(
                                                    protocol::AnyResponse::OnLoad(response),
                                                ),
                                            })
                                            .await;
                                    }
                                    Ok(None) => {
                                        let response = protocol::OnLoadResponse {
                                            id: None,
                                            plugin_name: None,
                                            errors: None,
                                            warnings: None,
                                            contents: None,
                                            resolve_dir: None,
                                            loader: None,
                                            plugin_data: None,
                                            watch_files: None,
                                            watch_dirs: None,
                                        };
                                        let _ = response_tx
                                            .send(protocol::ProtocolPacket {
                                                id,
                                                is_request: false,
                                                value: protocol::ProtocolMessage::Response(
                                                    protocol::AnyResponse::OnLoad(response),
                                                ),
                                            })
                                            .await;
                                    }
                                    Err(e) => eprintln!("Error on-load: {}", e),
                                }
                            });
                            return Ok(());
                        }
                        _ => {
                            todo!("handle: {:?}", packet.value)
                        }
                    },
                    _ => {
                        todo!("handle: {:?}", packet.value)
                    }
                }
            } else {
                let req_id = packet.id;
                let kind = pending.lock().remove(&req_id).unwrap();
                match kind {
                    protocol::RequestKind::Build(tx) => {
                        let build_response =
                            protocol::BuildResponse::from_any_value(packet.value.clone())?;
                        let _ = tx.send(build_response);
                    }
                }

                Ok(())
            }
        }
        _ => {
            todo!("handle: {:?}", packet.value)
        }
    }
}

type PendingResponseMap = HashMap<u32, protocol::RequestKind>;

pub struct ProtocolClientInner {
    response_tx: mpsc::Sender<protocol::ProtocolPacket>,
    id: AtomicU32,
    pending: Arc<Mutex<PendingResponseMap>>,
}

#[derive(Clone)]
pub struct ProtocolClient(Arc<ProtocolClientInner>);

impl ProtocolClient {
    pub fn new(response_tx: mpsc::Sender<protocol::ProtocolPacket>) -> Self {
        Self(Arc::new(ProtocolClientInner::new(response_tx)))
    }
}

impl Deref for ProtocolClient {
    type Target = ProtocolClientInner;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ProtocolClientInner {
    fn new(response_tx: mpsc::Sender<protocol::ProtocolPacket>) -> Self {
        Self {
            response_tx,
            id: AtomicU32::new(0),
            pending: Default::default(),
        }
    }

    pub async fn send_build_request(
        &self,
        req: protocol::BuildRequest,
    ) -> Result<protocol::BuildResponse, AnyError> {
        let id = self.id.fetch_add(1, Ordering::Relaxed);
        let packet = protocol::ProtocolPacket {
            id,
            is_request: true,
            value: protocol::ProtocolMessage::Request(protocol::AnyRequest::Build(req)),
        };
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .insert(id, protocol::RequestKind::Build(tx));
        self.response_tx.send(packet).await?;
        let response = rx.await?;
        Ok(response)
    }
}

#[derive(Clone, Debug, Copy)]
pub enum PackagesHandling {
    Bundle,
    External,
}

#[derive(Clone, Debug, Copy)]
pub enum Platform {
    Node,
    Browser,
    Neutral,
}

#[derive(Clone, Debug, Copy)]
pub enum Format {
    Esm,
    Cjs,
    Iife,
}

#[derive(Clone, Debug, Copy)]
pub enum LogLevel {
    Silent,
    Error,
    Warning,
    Info,
    Debug,
    Verbose,
}

#[derive(Clone, Debug, Copy)]
pub enum BuiltinLoader {
    Js,
    Ts,
    Jsx,
    Json,
    Css,
    Text,
    Binary,
    Base64,
    DataUrl,
    File,
    Copy,
    Empty,
}

#[derive(Clone, Debug)]
pub enum Loader {
    Builtin(BuiltinLoader),
    Map(IndexMap<String, BuiltinLoader>),
}

#[derive(derive_builder::Builder)]
#[builder(setter(strip_option), default)]
pub struct EsbuildFlags {
    color: Option<bool>,
    log_level: Option<LogLevel>,
    log_limit: Option<u32>,
    format: Option<Format>,
    platform: Option<Platform>,
    tree_shaking: Option<bool>,
    bundle: Option<bool>,
    outfile: Option<String>,
    packages: Option<PackagesHandling>,
    tsconfig: Option<String>,
    tsconfig_raw: Option<String>,
    loader: Option<IndexMap<String, BuiltinLoader>>,
}
fn default<T: Default>() -> T {
    T::default()
}

impl Default for EsbuildFlags {
    fn default() -> Self {
        Self {
            color: default(),
            log_level: default(),
            log_limit: default(),
            format: Some(Format::Esm),
            platform: Some(Platform::Node),
            tree_shaking: Some(true),
            bundle: Some(true),
            outfile: default(),
            packages: Some(PackagesHandling::Bundle),
            tsconfig: default(),
            tsconfig_raw: default(),
            loader: default(),
        }
    }
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Silent => write!(f, "silent"),
            LogLevel::Error => write!(f, "error"),
            LogLevel::Warning => write!(f, "warning"),
            LogLevel::Info => write!(f, "info"),
            LogLevel::Debug => write!(f, "debug"),
            LogLevel::Verbose => write!(f, "verbose"),
        }
    }
}

impl Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Format::Iife => write!(f, "iife"),
            Format::Cjs => write!(f, "cjs"),
            Format::Esm => write!(f, "esm"),
        }
    }
}

impl Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Platform::Browser => write!(f, "browser"),
            Platform::Node => write!(f, "node"),
            Platform::Neutral => write!(f, "neutral"),
        }
    }
}

impl Display for PackagesHandling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackagesHandling::Bundle => write!(f, "bundle"),
            PackagesHandling::External => write!(f, "external"),
        }
    }
}

impl Display for BuiltinLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuiltinLoader::Js => write!(f, "js"),
            BuiltinLoader::Jsx => write!(f, "jsx"),
            BuiltinLoader::Ts => write!(f, "ts"),
            BuiltinLoader::Json => write!(f, "json"),
            BuiltinLoader::Css => write!(f, "css"),
            BuiltinLoader::Text => write!(f, "text"),
            BuiltinLoader::Base64 => write!(f, "base64"),
            BuiltinLoader::DataUrl => write!(f, "dataurl"),
            BuiltinLoader::File => write!(f, "file"),
            BuiltinLoader::Binary => write!(f, "binary"),
            BuiltinLoader::Copy => write!(f, "copy"),
            BuiltinLoader::Empty => write!(f, "empty"),
        }
    }
}

impl EsbuildFlags {
    pub fn to_flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        if let Some(color) = self.color {
            flags.push(format!("--color={}", color));
        }
        if let Some(log_level) = self.log_level {
            flags.push(format!("--log-level={}", log_level));
        }
        if let Some(log_limit) = self.log_limit {
            flags.push(format!("--log-limit={}", log_limit));
        }
        if let Some(format) = self.format {
            flags.push(format!("--format={}", format));
        }
        if let Some(platform) = self.platform {
            flags.push(format!("--platform={}", platform));
        }
        if let Some(tree_shaking) = self.tree_shaking {
            flags.push(format!("--tree-shaking={}", tree_shaking));
        }
        if let Some(bundle) = self.bundle {
            flags.push(format!("--bundle={}", bundle));
        }
        if let Some(outfile) = &self.outfile {
            flags.push(format!("--outfile={}", outfile));
        }
        if let Some(packages) = self.packages {
            flags.push(format!("--packages={}", packages));
        }
        if let Some(tsconfig) = &self.tsconfig {
            flags.push(format!("--tsconfig={}", tsconfig));
        }
        if let Some(tsconfig_raw) = &self.tsconfig_raw {
            flags.push(format!("--tsconfig-raw={}", tsconfig_raw));
        }
        if let Some(loader) = &self.loader {
            for (key, value) in loader {
                flags.push(format!("--loader={}={}", key, value));
            }
        }
        flags
    }
}
