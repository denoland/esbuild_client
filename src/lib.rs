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
    AnyPacket, AnyValue, Encode, FromAnyValue, FromMap, ImportKind, PartialMessage, ProtocolPacket,
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
        buffer.extend(std::iter::repeat_n(0, 1024));
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
        state.buffer.extend(std::iter::repeat_n(0, 1024));
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
                    log::trace!("decoded packet: {packet:?}");
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
                log::trace!("got send packet from receiver: {packet:?}");
                let mut encoded = Vec::new();
                packet.encode_into(&mut encoded);
                stdin.write_all(&encoded).await?;
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

pub struct NoopPluginHandler;
#[async_trait(?Send)]
impl PluginHandler for NoopPluginHandler {
    async fn on_resolve(&self, _args: OnResolveArgs) -> Result<Option<OnResolveResult>, AnyError> {
        Ok(None)
    }
    async fn on_load(&self, _args: OnLoadArgs) -> Result<Option<OnLoadResult>, AnyError> {
        Ok(None)
    }
    async fn on_start(&self, _args: OnStartArgs) -> Result<Option<OnStartResult>, AnyError> {
        Ok(None)
    }
    async fn on_end(&self, _args: OnEndArgs) -> Result<Option<OnEndResult>, AnyError> {
        Ok(None)
    }
}

impl MakePluginHandler for Option<()> {
    fn make_plugin_handler(self, _client: ProtocolClient) -> Arc<dyn PluginHandler> {
        Arc::new(NoopPluginHandler)
    }
}

#[derive(Default)]
pub struct EsbuildServiceOptions<'a> {
    pub cwd: Option<&'a Path>,
}

impl EsbuildService {
    pub async fn new(
        path: impl AsRef<Path>,
        version: &str,
        plugin_handler: impl MakePluginHandler,
        options: EsbuildServiceOptions<'_>,
    ) -> Result<Self, AnyError> {
        let path = path.as_ref();
        let mut cmd = tokio::process::Command::new(path);
        if let Some(cwd) = options.cwd {
            cmd.current_dir(cwd);
        }
        let mut esbuild = cmd
            .arg(format!("--service={}", version))
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
        deno_unsync::spawn(protocol_task(
            stdout,
            stdin,
            ready_tx,
            response_rx,
            packet_tx,
        ));

        deno_unsync::spawn(async move {
            let status = esbuild.wait().await.unwrap();
            let _ = exited_tx.send(Some(status));
        });

        let client = ProtocolClient::new(response_tx.clone());
        let plugin_handler = plugin_handler.make_plugin_handler(client.clone());
        let pending = client.0.pending.clone();

        deno_unsync::spawn(async move {
            loop {
                let packet = packet_rx.recv().await;

                log::trace!("got packet from receiver: {packet:?}");
                if let Some(packet) = packet {
                    let _ = handle_packet(packet, &response_tx, plugin_handler.clone(), &pending)
                        .await
                        .inspect_err(|err| {
                            eprintln!("failed to handle packet {err}");
                        });
                } else {
                    break;
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

fn resolve_result_to_response(
    id: u32,
    result: Option<OnResolveResult>,
) -> protocol::OnResolveResponse {
    match result {
        Some(result) => protocol::OnResolveResponse {
            id: Some(id),
            plugin_name: result.plugin_name,
            errors: result.errors,
            warnings: result.warnings,
            path: result.path,
            external: result.external,
            side_effects: result.side_effects,
            namespace: result.namespace,
            suffix: result.suffix,
            plugin_data: result.plugin_data,
            watch_files: result.watch_files,
            watch_dirs: result.watch_dirs,
        },
        None => protocol::OnResolveResponse {
            id: Some(id),
            ..Default::default()
        },
    }
}

fn load_result_to_response(id: u32, result: Option<OnLoadResult>) -> protocol::OnLoadResponse {
    match result {
        Some(result) => protocol::OnLoadResponse {
            id: result.id,
            plugin_name: result.plugin_name,
            errors: result.errors,
            warnings: result.warnings,
            contents: result.contents,
            resolve_dir: result.resolve_dir,
            loader: result.loader.map(|loader| loader.to_string()),
            plugin_data: result.plugin_data,
            watch_files: result.watch_files,
            watch_dirs: result.watch_dirs,
        },
        None => protocol::OnLoadResponse {
            id: Some(id),
            ..Default::default()
        },
    }
}

#[async_trait(?Send)]
pub trait PluginHandler {
    async fn on_resolve(&self, _args: OnResolveArgs) -> Result<Option<OnResolveResult>, AnyError>;
    async fn on_load(&self, _args: OnLoadArgs) -> Result<Option<OnLoadResult>, AnyError>;
    async fn on_start(&self, _args: OnStartArgs) -> Result<Option<OnStartResult>, AnyError>;
    async fn on_end(&self, _args: OnEndArgs) -> Result<Option<OnEndResult>, AnyError>;
}

#[derive(Default)]
pub struct OnStartResult {
    pub errors: Option<Vec<PartialMessage>>,
    pub warnings: Option<Vec<PartialMessage>>,
}

#[derive(Default)]
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

impl std::fmt::Debug for OnLoadResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnLoadResult")
            .field("id", &self.id)
            .field("plugin_name", &self.plugin_name)
            .field("errors", &self.errors)
            .field("warnings", &self.warnings)
            .field(
                "contents",
                &self.contents.as_ref().map(|c| String::from_utf8_lossy(c)),
            )
            .field("resolve_dir", &self.resolve_dir)
            .field("loader", &self.loader)
            .field("plugin_data", &self.plugin_data)
            .field("watch_files", &self.watch_files)
            .field("watch_dirs", &self.watch_dirs)
            .finish()
    }
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

#[derive(Debug)]
pub struct OnStartArgs {
    pub key: u32,
}

trait PluginHook {
    type Request: FromMap;
    type Response: Into<protocol::AnyResponse>;
    type Result;
    type Args: From<Self::Request>;
    fn call_handler(
        plugin_handler: Arc<dyn PluginHandler>,
        args: Self::Args,
    ) -> impl Future<Output = Result<Option<Self::Result>, AnyError>>;

    fn response_from_result(
        id: u32,
        result: Result<Option<Self::Result>, AnyError>,
    ) -> Self::Response;
}

struct OnResolveHook;
impl PluginHook for OnResolveHook {
    type Request = protocol::OnResolveRequest;
    type Response = protocol::OnResolveResponse;
    type Args = OnResolveArgs;
    type Result = OnResolveResult;
    async fn call_handler(
        plugin_handler: Arc<dyn PluginHandler>,
        args: Self::Args,
    ) -> Result<Option<Self::Result>, AnyError> {
        plugin_handler.on_resolve(args).await
    }
    fn response_from_result(
        id: u32,
        result: Result<Option<Self::Result>, AnyError>,
    ) -> Self::Response {
        match result {
            Ok(result) => resolve_result_to_response(id, result),
            Err(e) => {
                log::debug!("error calling on-resolve: {}", e);
                protocol::OnResolveResponse::default()
            }
        }
    }
}

impl From<protocol::OnResolveRequest> for OnResolveArgs {
    fn from(on_resolve: protocol::OnResolveRequest) -> Self {
        fn empty_none(s: String) -> Option<String> {
            if s.is_empty() { None } else { Some(s) }
        }
        OnResolveArgs {
            key: on_resolve.key,
            ids: on_resolve.ids,
            path: on_resolve.path,
            importer: empty_none(on_resolve.importer),
            kind: on_resolve.kind,
            namespace: empty_none(on_resolve.namespace),
            resolve_dir: on_resolve.resolve_dir,
            with: on_resolve.with,
        }
    }
}

struct OnLoadHook;
impl PluginHook for OnLoadHook {
    type Request = protocol::OnLoadRequest;
    type Response = protocol::OnLoadResponse;
    type Args = OnLoadArgs;
    type Result = OnLoadResult;
    async fn call_handler(
        plugin_handler: Arc<dyn PluginHandler>,
        args: Self::Args,
    ) -> Result<Option<Self::Result>, AnyError> {
        plugin_handler.on_load(args).await
    }
    fn response_from_result(
        id: u32,
        result: Result<Option<Self::Result>, AnyError>,
    ) -> Self::Response {
        match result {
            Ok(result) => load_result_to_response(id, result),
            Err(e) => {
                log::debug!("error calling on-load: {}", e);
                protocol::OnLoadResponse::default()
            }
        }
    }
}
impl From<protocol::OnLoadRequest> for OnLoadArgs {
    fn from(on_load: protocol::OnLoadRequest) -> Self {
        OnLoadArgs {
            key: on_load.key,
            path: on_load.path,
            ids: on_load.ids,
            namespace: on_load.namespace,
            suffix: on_load.suffix,
            plugin_data: on_load.plugin_data,
            with: on_load.with,
        }
    }
}

struct OnStartHook;
impl PluginHook for OnStartHook {
    type Request = protocol::OnStartRequest;
    type Response = protocol::OnStartResponse;
    type Args = OnStartArgs;
    type Result = OnStartResult;
    async fn call_handler(
        plugin_handler: Arc<dyn PluginHandler>,
        args: Self::Args,
    ) -> Result<Option<Self::Result>, AnyError> {
        plugin_handler.on_start(args).await
    }
    fn response_from_result(
        _id: u32,
        result: Result<Option<Self::Result>, AnyError>,
    ) -> Self::Response {
        match result {
            Ok(Some(result)) => protocol::OnStartResponse {
                errors: result.errors.unwrap_or_default(),
                warnings: result.warnings.unwrap_or_default(),
            },
            Ok(None) => protocol::OnStartResponse::default(),
            Err(e) => {
                log::debug!("error calling on-start: {}", e);
                protocol::OnStartResponse::default()
            }
        }
    }
}

impl From<protocol::OnStartRequest> for OnStartArgs {
    fn from(on_start: protocol::OnStartRequest) -> Self {
        OnStartArgs { key: on_start.key }
    }
}

#[derive(Debug)]
pub struct OnEndArgs {
    pub errors: Vec<protocol::Message>,
    pub warnings: Vec<protocol::Message>,
    pub output_files: Option<Vec<protocol::BuildOutputFile>>,
    pub metafile: Option<String>,
    pub mangle_cache: Option<IndexMap<String, protocol::MangleCacheEntry>>,
    pub write_to_stdout: Option<Vec<u8>>,
}

impl From<OnEndArgs> for protocol::BuildResponse {
    fn from(end: OnEndArgs) -> Self {
        Self {
            errors: end.errors,
            warnings: end.warnings,
            output_files: end.output_files,
            metafile: end.metafile,
            mangle_cache: end.mangle_cache,
            write_to_stdout: end.write_to_stdout,
        }
    }
}

struct OnEndHook;
impl PluginHook for OnEndHook {
    type Request = protocol::OnEndRequest;
    type Response = protocol::OnEndResponse;
    type Args = OnEndArgs;
    type Result = OnEndResult;
    async fn call_handler(
        plugin_handler: Arc<dyn PluginHandler>,
        args: Self::Args,
    ) -> Result<Option<Self::Result>, AnyError> {
        plugin_handler.on_end(args).await
    }
    fn response_from_result(
        _id: u32,
        _result: Result<Option<Self::Result>, AnyError>,
    ) -> Self::Response {
        match _result {
            Ok(Some(result)) => protocol::OnEndResponse {
                errors: result.errors.unwrap_or_default(),
                warnings: result.warnings.unwrap_or_default(),
            },
            Ok(None) => protocol::OnEndResponse::default(),
            Err(e) => {
                log::debug!("error calling on-end: {}", e);
                protocol::OnEndResponse::default()
            }
        }
    }
}

#[derive(Default)]
pub struct OnEndResult {
    pub errors: Option<Vec<protocol::Message>>,
    pub warnings: Option<Vec<protocol::Message>>,
}

impl From<protocol::OnEndRequest> for OnEndArgs {
    fn from(value: protocol::OnEndRequest) -> Self {
        OnEndArgs {
            errors: value.errors,
            warnings: value.warnings,
            output_files: value.output_files,
            metafile: value.metafile,
            mangle_cache: value.mangle_cache,
            write_to_stdout: value.write_to_stdout,
        }
    }
}

async fn handle_hook<H: PluginHook>(
    id: u32,
    request: H::Request,
    response_tx: &mpsc::Sender<protocol::ProtocolPacket>,
    plugin_handler: Arc<dyn PluginHandler>,
) -> Result<(), AnyError> {
    let args = H::Args::from(request);
    let result = H::call_handler(plugin_handler, args).await;
    let response = H::response_from_result(id, result);
    response_tx
        .send(protocol::ProtocolPacket {
            id,
            is_request: false,
            value: protocol::ProtocolMessage::Response(response.into()),
        })
        .await?;

    Ok(())
}

fn spawn_hook<H: PluginHook>(
    id: u32,
    map: &IndexMap<String, AnyValue>,
    response_tx: &mpsc::Sender<protocol::ProtocolPacket>,
    plugin_handler: Arc<dyn PluginHandler>,
) -> Result<deno_unsync::JoinHandle<Result<(), AnyError>>, AnyError>
where
    H::Request: 'static,
{
    let request = H::Request::from_map(map)?;
    let response_tx = response_tx.clone();
    let plugin_handler = plugin_handler.clone();
    Ok(deno_unsync::spawn(async move {
        handle_hook::<H>(id, request, &response_tx, plugin_handler).await?;
        Ok(())
    }))
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
                            handle_hook::<OnStartHook>(
                                packet.id,
                                protocol::OnStartRequest::from_map(index_map)?,
                                response_tx,
                                plugin_handler,
                            )
                            .await?;
                            Ok(())
                        }
                        "on-resolve" => {
                            spawn_hook::<OnResolveHook>(
                                packet.id,
                                index_map,
                                response_tx,
                                plugin_handler,
                            )?;

                            Ok(())
                        }
                        "on-load" => {
                            spawn_hook::<OnLoadHook>(
                                packet.id,
                                index_map,
                                response_tx,
                                plugin_handler,
                            )?;
                            Ok(())
                        }
                        "on-end" => {
                            spawn_hook::<OnEndHook>(
                                packet.id,
                                index_map,
                                response_tx,
                                plugin_handler,
                            )?;
                            Ok(())
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
                    protocol::RequestKind::Dispose(tx) => {
                        let _ = tx.send(());
                    }
                    protocol::RequestKind::Rebuild(tx) => {
                        let rebuild_response =
                            protocol::RebuildResponse::from_any_value(packet.value.clone())?;
                        let _ = tx.send(rebuild_response);
                    }
                    protocol::RequestKind::Cancel(tx) => {
                        let _ = tx.send(());
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
            value: protocol::ProtocolMessage::Request(protocol::AnyRequest::Build(Box::new(req))),
        };
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .insert(id, protocol::RequestKind::Build(tx));
        self.response_tx.send(packet).await?;
        let response = rx.await?;
        Ok(response)
    }

    pub async fn send_dispose_request(&self, key: u32) -> Result<(), AnyError> {
        let id = self.id.fetch_add(1, Ordering::Relaxed);
        let packet = protocol::ProtocolPacket {
            id,
            is_request: true,
            value: protocol::ProtocolMessage::Request(protocol::AnyRequest::Dispose(
                protocol::DisposeRequest { key },
            )),
        };
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .insert(id, protocol::RequestKind::Dispose(tx));
        self.response_tx.send(packet).await?;
        rx.await?;
        Ok(())
    }

    pub async fn send_rebuild_request(
        &self,
        key: u32,
    ) -> Result<protocol::RebuildResponse, AnyError> {
        let id = self.id.fetch_add(1, Ordering::Relaxed);
        let packet = protocol::ProtocolPacket {
            id,
            is_request: true,
            value: protocol::ProtocolMessage::Request(protocol::AnyRequest::Rebuild(
                protocol::RebuildRequest { key },
            )),
        };
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .insert(id, protocol::RequestKind::Rebuild(tx));
        self.response_tx.send(packet).await?;
        let response = rx.await?;
        Ok(response)
    }

    pub async fn send_cancel_request(&self, key: u32) -> Result<(), AnyError> {
        let id = self.id.fetch_add(1, Ordering::Relaxed);
        let packet = protocol::ProtocolPacket {
            id,
            is_request: true,
            value: protocol::ProtocolMessage::Request(protocol::AnyRequest::Cancel(
                protocol::CancelRequest { key },
            )),
        };
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .insert(id, protocol::RequestKind::Cancel(tx));
        self.response_tx.send(packet).await?;
        rx.await?;
        Ok(())
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
    outdir: Option<String>,
    packages: Option<PackagesHandling>,
    tsconfig: Option<String>,
    tsconfig_raw: Option<String>,
    loader: Option<IndexMap<String, BuiltinLoader>>,
    external: Option<Vec<String>>,
    minify: Option<bool>,
    splitting: Option<bool>,
    metafile: Option<bool>,
    sourcemap: Option<Sourcemap>,
    defines: Option<IndexMap<String, String>>,
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
            outdir: default(),
            packages: Some(PackagesHandling::Bundle),
            tsconfig: default(),
            tsconfig_raw: default(),
            loader: default(),
            external: default(),
            minify: default(),
            splitting: default(),
            metafile: default(),
            sourcemap: default(),
            defines: default(),
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

#[derive(Clone, Debug, Copy)]
pub enum Sourcemap {
    Linked,
    External,
    Inline,
    Both,
}

impl Display for Sourcemap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Sourcemap::Linked => write!(f, "linked"),
            Sourcemap::External => write!(f, "external"),
            Sourcemap::Inline => write!(f, "inline"),
            Sourcemap::Both => write!(f, "both"),
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
        if let Some(outdir) = &self.outdir {
            flags.push(format!("--outdir={}", outdir));
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
                flags.push(format!("--loader:{}={}", key, value));
            }
        }
        if let Some(external) = &self.external {
            for external in external {
                flags.push(format!("--external:{}", external));
            }
        }
        if let Some(minify) = self.minify {
            if minify {
                flags.push("--minify".to_string());
            }
        }
        if let Some(splitting) = self.splitting {
            if splitting {
                flags.push("--splitting".to_string());
            }
        }
        if let Some(defines) = &self.defines {
            for (key, value) in defines {
                flags.push(format!("--define:{}={}", key, value));
            }
        }
        if let Some(metafile) = self.metafile {
            if metafile {
                flags.push("--metafile".to_string());
            }
        }
        if let Some(sourcemap) = self.sourcemap {
            flags.push(format!("--sourcemap={}", sourcemap));
        }
        flags
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[derive(Clone, Debug)]
pub struct Metafile {
    #[cfg_attr(feature = "serde", serde(default))]
    pub inputs: HashMap<String, MetafileInput>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub outputs: HashMap<String, MetafileOutput>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[derive(Clone, Debug)]
pub struct MetafileInput {
    pub bytes: u64,
    pub imports: Vec<MetafileInputImport>,
    pub format: Option<String>,
    pub with: Option<HashMap<String, String>>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[derive(Clone, Debug)]
pub struct MetafileInputImport {
    pub path: String,
    pub kind: ImportKind,
    #[cfg_attr(feature = "serde", serde(default))]
    pub external: bool,
    pub original: Option<String>,
    pub with: Option<HashMap<String, String>>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[derive(Clone, Debug)]
pub struct MetafileOutput {
    pub bytes: u64,
    pub inputs: HashMap<String, MetafileOutputInput>,
    pub imports: Vec<MetafileOutputImport>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub exports: Vec<String>,
    pub entry_point: Option<String>,
    pub css_bundle: Option<String>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[derive(Clone, Debug)]
pub struct MetafileOutputInput {
    pub bytes_in_output: u64,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[derive(Clone, Debug)]
pub struct MetafileOutputImport {
    pub path: String,
    pub kind: ImportKind,
    #[cfg_attr(feature = "serde", serde(default))]
    pub external: Option<bool>,
}
