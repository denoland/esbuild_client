pub mod encode;

use anyhow::Context;
use std::{fmt::Debug, hash::Hash};
use tokio::sync::oneshot;

use crate::{impl_encode_command, impl_encode_struct};
#[allow(unused_imports)]
use encode::encode_length_delimited;
pub use encode::{Encode, encode_key, encode_u32_raw};
use indexmap::IndexMap;

macro_rules! get {
    ($self:expr, $key:expr) => {
        $self
            .get($key)
            .ok_or_else(|| anyhow::anyhow!("Missing field: {}", $key))
            .and_then(|v| v.clone().to_type())
            .context(format!("on key: {}", $key))
    };
}

macro_rules! impl_from_map {
    (for $t: ty { $( $(#[$optional:ident])? $field: ident),* }) => {
        impl FromMap for $t {
            fn from_map(map: &IndexMap<String, AnyValue>) -> Result<Self, anyhow::Error> {
                $(
                    $crate::protocol::impl_from_map!(@helper; map; $($optional)? $field);
                )*
                Ok(Self { $($field),* })
            }
        }
    };
    (@helper; $self: ident; optional $field: ident) => {
        let $field = if let Some(val) = $self.get(&encode::snake_to_camel(stringify!($field))) {
            val.clone().to_type()?
        } else {
            None
        };
    };
    (@helper; $self: ident; $field: ident) => {
        let $field = get!($self, &encode::snake_to_camel(stringify!($field)))?;
    };
}

macro_rules! protocol_impls {
    (for $t: ty { $( $(#[$optional:ident])? $field: ident),* }) => {
        impl_from_map!(for $t { $( $(#[$optional])? $field),* });
        impl_encode_struct!(for $t { $($field),* });
    };
}

macro_rules! enum_impl_from {
    (for $t: ty { $( $variant:ident($field: ty)),*  $(,)? }) => {
        $(
            impl From<$field> for $t {
                fn from(value: $field) -> Self {
                    <$t>::$variant(value)
                }
            }

        )*
    };
}

pub(crate) use impl_from_map;

pub trait Decode {
    fn decode_from<'a>(buf: &mut Buf<'a>) -> Result<Self, anyhow::Error>
    where
        Self: Sized;
}

#[derive(Debug, Clone)]
pub struct Buf<'a> {
    buf: &'a [u8],
    idx: usize,
}

impl<'a> Buf<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, idx: 0 }
    }

    fn read_u8(&mut self) -> u8 {
        let value = self.buf[self.idx];
        self.idx += 1;
        value
    }

    fn read_n(&mut self, n: usize, into: &mut [u8]) {
        into.copy_from_slice(&self.buf[self.idx..self.idx + n]);
        self.idx += n;
    }

    fn read_u32(&mut self) -> u32 {
        let value = u32::from_le_bytes(self.buf[self.idx..self.idx + 4].try_into().unwrap());
        self.idx += 4;
        value
    }
}

pub trait FromAnyValue: Sized {
    fn from_any_value(value: AnyValue) -> Result<Self, anyhow::Error>;
}

impl FromAnyValue for AnyValue {
    fn from_any_value(value: AnyValue) -> Result<Self, anyhow::Error> {
        Ok(value)
    }
}

impl FromAnyValue for String {
    fn from_any_value(value: AnyValue) -> Result<Self, anyhow::Error> {
        match value {
            AnyValue::String(s) => Ok(s),
            _ => Err(anyhow::anyhow!("expected string")),
        }
    }
}

impl FromAnyValue for u32 {
    fn from_any_value(value: AnyValue) -> Result<Self, anyhow::Error> {
        match value {
            AnyValue::U32(u) => Ok(u),
            _ => Err(anyhow::anyhow!("expected u32, got {value:?}")),
        }
    }
}

impl FromAnyValue for bool {
    fn from_any_value(value: AnyValue) -> Result<Self, anyhow::Error> {
        match value {
            AnyValue::Bool(b) => Ok(b),
            _ => Err(anyhow::anyhow!("expected bool")),
        }
    }
}

impl FromAnyValue for Vec<u8> {
    fn from_any_value(value: AnyValue) -> Result<Self, anyhow::Error> {
        match value {
            AnyValue::Bytes(b) => Ok(b),
            _ => Err(anyhow::anyhow!("expected bytes")),
        }
    }
}

impl<T: FromAnyValue> FromAnyValue for Vec<T> {
    fn from_any_value(value: AnyValue) -> Result<Self, anyhow::Error> {
        match value {
            AnyValue::Vec(v) => Ok(v
                .into_iter()
                .map(T::from_any_value)
                .collect::<Result<Vec<_>, _>>()?),
            _ => Err(anyhow::anyhow!("expected vec")),
        }
    }
}

impl<V: FromAnyValue> FromAnyValue for IndexMap<String, V> {
    fn from_any_value(value: AnyValue) -> Result<Self, anyhow::Error> {
        match value {
            AnyValue::Map(m) => {
                let mut map = IndexMap::new();
                for (k, v) in m {
                    map.insert(k, V::from_any_value(v)?);
                }
                Ok(map)
            }
            _ => Err(anyhow::anyhow!("expected map")),
        }
    }
}

impl<T: FromAnyValue> FromAnyValue for Option<T> {
    fn from_any_value(value: AnyValue) -> Result<Self, anyhow::Error> {
        match value {
            AnyValue::Null => Ok(None),
            _ => Ok(Some(T::from_any_value(value)?)),
        }
    }
}

impl<T> FromAnyValue for T
where
    T: FromMap,
{
    fn from_any_value(value: AnyValue) -> Result<Self, anyhow::Error> {
        Self::from_map(value.as_map()?)
    }
}

#[derive(Debug, Clone)]
pub struct Packet<T> {
    pub id: u32,
    pub is_request: bool,
    pub value: T,
}

#[derive(Debug, Clone, Default)]
pub struct BuildRequest {
    pub key: u32,
    pub entries: Vec<(String, String)>,
    pub flags: Vec<String>,
    pub write: bool,
    pub stdin_contents: OptionNull<Vec<u8>>,
    pub stdin_resolve_dir: OptionNull<String>,
    pub abs_working_dir: String,
    pub node_paths: Vec<String>,
    pub context: bool,
    pub plugins: Option<Vec<BuildPlugin>>,
    pub mangle_cache: Option<IndexMap<String, MangleCacheEntry>>,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: String,
    pub plugin_name: String,
    pub text: String,
    pub location: Option<Location>,
    pub notes: Vec<Note>,
    pub detail: Option<AnyValue>,
    // detail: any
}

protocol_impls!(for Message { id, plugin_name, text, #[optional] location, notes, #[optional] detail });

#[derive(Debug, Clone)]
pub struct Note {
    pub text: String,
    pub location: Option<Location>,
}
protocol_impls!(for Note { text, #[optional] location });

#[derive(Debug, Clone)]
pub struct Location {
    pub file: String,
    pub namespace: String,
    pub line: u32,
    pub column: u32,
    pub length: u32,
    pub line_text: String,
    pub suggestion: String,
}

protocol_impls!(for Location { file, namespace, line, column, length, line_text, suggestion });

#[derive(Debug, Clone, Default)]
pub struct PartialMessage {
    pub id: String,
    pub plugin_name: String,
    pub text: String,
    pub location: Option<Location>,
    pub notes: Vec<Note>,
    pub detail: u32,
}

protocol_impls!(for PartialMessage { id, plugin_name, text, #[optional] location, notes, detail });

#[derive(Debug, Clone)]
pub struct ServeOnRequestArgs {
    pub remote_address: String,
    pub method: String,
    pub path: String,
    pub status: u32,
    /// The time to generate the response, not to send it
    pub time_in_ms: u32,
}

impl_encode_struct!(for ServeOnRequestArgs { remote_address, method, path, status, time_in_ms });

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum ImportKind {
    EntryPoint,
    ImportStatement,
    RequireCall,
    DynamicImport,
    RequireResolve,
    // Css
    ImportRule,
    ComposesFrom,
    UrlToken,
}

impl FromAnyValue for ImportKind {
    fn from_any_value(value: AnyValue) -> Result<Self, anyhow::Error> {
        let s = value.as_string()?;
        Ok(match s.as_str() {
            "entry-point" => ImportKind::EntryPoint,
            "import-statement" => ImportKind::ImportStatement,
            "require-call" => ImportKind::RequireCall,
            "dynamic-import" => ImportKind::DynamicImport,
            "require-resolve" => ImportKind::RequireResolve,
            "import-rule" => ImportKind::ImportRule,
            "composes-from" => ImportKind::ComposesFrom,
            "url-token" => ImportKind::UrlToken,
            _ => return Err(anyhow::anyhow!("invalid import kind: {}", s)),
        })
    }
}
// Equivalent to TypeScript MangleCacheEntry: string | false
#[derive(Debug, Clone)]
pub enum MangleCacheEntry {
    StringValue(String),
    BoolValue(bool),
}

impl FromAnyValue for MangleCacheEntry {
    fn from_any_value(value: AnyValue) -> Result<Self, anyhow::Error> {
        match value {
            AnyValue::Bool(b) => Ok(MangleCacheEntry::BoolValue(b)),
            AnyValue::String(s) => Ok(MangleCacheEntry::StringValue(s)),
            value => Err(anyhow::anyhow!("invalid mangle cache entry: {:?}", value)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServeRequest {
    // command: "serve"; // This will likely be handled by an enum or similar in a real IPC setup
    pub key: u32,
    pub on_request: bool,
    pub port: Option<u32>,
    pub host: Option<String>,
    pub servedir: Option<String>,
    pub keyfile: Option<String>,
    pub certfile: Option<String>,
    pub fallback: Option<String>,
    pub cors_origin: Option<Vec<String>>,
}

impl_encode_command!(for ServeRequest {
  const Command = "serve";
  key, on_request, port, host, servedir, keyfile, certfile, fallback, cors_origin
});

#[derive(Debug, Clone)]
pub struct ServeResponse {
    pub port: u32,
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BuildPlugin {
    pub name: String,
    pub on_start: bool,
    pub on_end: bool,
    pub on_resolve: Vec<OnResolveSetupOptions>,
    pub on_load: Vec<OnLoadSetupOptions>,
}

impl_encode_struct!(for BuildPlugin { name, on_start, on_end, on_resolve, on_load });

#[derive(Debug, Clone)]
pub struct OnResolveSetupOptions {
    pub id: u32,
    pub filter: String,
    pub namespace: String,
}

impl_encode_struct!(for OnResolveSetupOptions { id, filter, namespace });

#[derive(Debug, Clone)]
pub struct OnLoadSetupOptions {
    pub id: u32,
    pub filter: String,
    pub namespace: String,
}

impl_encode_struct!(for OnLoadSetupOptions { id, filter, namespace });

#[derive(Debug, Clone)]
pub struct BuildResponse {
    pub errors: Vec<Message>,
    pub warnings: Vec<Message>,
    pub output_files: Option<Vec<BuildOutputFile>>,
    pub metafile: Option<String>,
    pub mangle_cache: Option<IndexMap<String, MangleCacheEntry>>,
    pub write_to_stdout: Option<Vec<u8>>,
}

impl_from_map!(for BuildResponse {
    errors, warnings, #[optional] output_files, #[optional] metafile, #[optional] mangle_cache, #[optional] write_to_stdout
});

#[derive(Debug, Clone)]
pub struct OnEndRequest {
    // Extends BuildResponse
    // command: "on-end";
    // Fields from BuildResponse
    pub errors: Vec<Message>,
    pub warnings: Vec<Message>,
    pub output_files: Option<Vec<BuildOutputFile>>,
    pub metafile: Option<String>,
    pub mangle_cache: Option<IndexMap<String, MangleCacheEntry>>,
    pub write_to_stdout: Option<Vec<u8>>,
}

protocol_impls!(for OnEndRequest { errors, warnings, #[optional] output_files, #[optional] metafile, #[optional] mangle_cache, #[optional] write_to_stdout });

#[derive(Debug, Clone, Default)]
pub struct OnEndResponse {
    pub errors: Vec<Message>,
    pub warnings: Vec<Message>,
}

impl_encode_struct!(for OnEndResponse { errors, warnings });

#[derive(Debug, Clone)]
pub struct BuildOutputFile {
    pub path: String,
    pub contents: Vec<u8>,
    pub hash: String,
}

protocol_impls!(for BuildOutputFile { path, contents, hash });

#[derive(Debug, Clone)]
pub struct PingRequest {
    // command: "ping";
}

#[derive(Debug, Clone)]
pub struct RebuildRequest {
    // command: "rebuild";
    pub key: u32,
}

impl_encode_command!(for RebuildRequest {
  const Command = "rebuild";
  key
});

#[derive(Debug, Clone)]
pub struct RebuildResponse {
    pub errors: Vec<Message>,
    pub warnings: Vec<Message>,
}

protocol_impls!(for RebuildResponse { errors, warnings });

#[derive(Debug, Clone)]
pub struct DisposeRequest {
    // command: "dispose";
    pub key: u32,
}

impl_encode_command!(for DisposeRequest {
  const Command = "dispose";
  key
});

#[derive(Debug, Clone)]
pub struct CancelRequest {
    // command: "cancel";
    pub key: u32,
}

impl_encode_command!(for CancelRequest {
  const Command = "cancel";
  key
});

#[derive(Debug, Clone)]
pub struct WatchRequest {
    // command: "watch";
    pub key: u32,
}

impl_encode_command!(for WatchRequest {
  const Command = "watch";
  key
});

#[derive(Debug, Clone)]
pub struct OnServeRequest {
    // command: "serve-request";
    pub key: u32,
    pub args: ServeOnRequestArgs,
}

impl_encode_command!(for OnServeRequest {
  const Command = "serve-request";
  key, args
});

#[derive(Debug, Clone)]
pub struct TransformRequest {
    // command: "transform";
    pub flags: Vec<String>,
    pub input: Vec<u8>,
    pub input_fs: bool,
    pub mangle_cache: Option<IndexMap<String, MangleCacheEntry>>,
}

impl_encode_command!(for TransformRequest {
  const Command = "transform";
  flags, input, input_fs, mangle_cache
});

#[derive(Debug, Clone)]
pub struct TransformResponse {
    pub errors: Vec<Message>,
    pub warnings: Vec<Message>,
    pub code: String,
    pub code_fs: bool,
    pub map: String,
    pub map_fs: bool,
    pub legal_comments: Option<String>,
    pub mangle_cache: Option<IndexMap<String, MangleCacheEntry>>,
}

#[derive(Debug, Clone)]
pub struct FormatMsgsRequest {
    // command: "format-msgs";
    pub messages: Vec<Message>,
    pub is_warning: bool,
    pub color: Option<bool>,
    pub terminal_width: Option<u32>,
}

impl_encode_command!(for FormatMsgsRequest {
  const Command = "format-msgs";
  messages, is_warning, color, terminal_width
});

#[derive(Debug, Clone)]
pub struct FormatMsgsResponse {
    pub messages: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AnalyzeMetafileRequest {
    // command: "analyze-metafile";
    pub metafile: String,
    pub color: Option<bool>,
    pub verbose: Option<bool>,
}

impl_encode_command!(for AnalyzeMetafileRequest {
  const Command = "analyze-metafile";
  metafile, color, verbose
});

#[derive(Debug, Clone)]
pub struct AnalyzeMetafileResponse {
    pub result: String,
}

#[derive(Debug, Clone)]
pub struct OnStartRequest {
    // command: "on-start";
    pub key: u32,
}

impl_encode_command!(for OnStartRequest {
  const Command = "on-start";
  key
});

#[derive(Debug, Clone, Default)]
pub struct OnStartResponse {
    pub errors: Vec<PartialMessage>,
    pub warnings: Vec<PartialMessage>,
}

impl_encode_struct!(for OnStartResponse { errors, warnings });

#[derive(Debug, Clone)]
pub struct ResolveRequest {
    // command: "resolve";
    pub key: u32,
    pub path: String,
    pub plugin_name: String,
    pub importer: Option<String>,
    pub namespace: Option<String>,
    pub resolve_dir: Option<String>,
    pub kind: Option<String>, // Consider using an enum if kinds are fixed
    pub plugin_data: Option<u32>, // Assuming u32 for opaque data, adjust if needed
    pub with: Option<IndexMap<String, String>>,
}

impl_encode_command!(for ResolveRequest {
  const Command = "resolve";
  key, path, plugin_name, importer, namespace, resolve_dir, kind, plugin_data, with
});

#[derive(Debug, Clone)]
pub struct ResolveResponse {
    pub errors: Vec<Message>,
    pub warnings: Vec<Message>,
    pub path: String,
    pub external: bool,
    pub side_effects: bool,
    pub namespace: String,
    pub suffix: String,
    pub plugin_data: u32, // Assuming u32 for opaque data
}

#[derive(Debug, Clone)]
pub struct OnResolveRequest {
    // command: "on-resolve";
    pub key: u32,
    pub ids: Vec<u32>,
    pub path: String,
    pub importer: String,
    pub namespace: String,
    pub resolve_dir: Option<String>,
    pub kind: ImportKind,
    #[allow(dead_code)]
    pub plugin_data: Option<u32>,
    pub with: IndexMap<String, String>,
}

impl_from_map!(for OnResolveRequest {
    key, ids, path, importer, namespace, #[optional] resolve_dir, kind, #[optional] plugin_data, with
});

#[derive(Clone, Default)]
pub struct OptionNull<T>(Option<T>);

impl<T> OptionNull<T> {
    pub fn new(value: Option<T>) -> Self {
        Self(value)
    }
}

impl<T> From<OptionNull<T>> for Option<T> {
    fn from(value: OptionNull<T>) -> Self {
        value.0
    }
}

impl<T> From<Option<T>> for OptionNull<T> {
    fn from(value: Option<T>) -> Self {
        Self(value)
    }
}

impl<T: Debug> Debug for OptionNull<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

#[derive(Debug, Clone, Default)]
pub struct OnResolveResponse {
    // TODO(nathanwhit): plugin id or request id or ??
    pub id: Option<u32>,
    pub plugin_name: Option<String>,
    pub errors: Option<Vec<PartialMessage>>,
    pub warnings: Option<Vec<PartialMessage>>,
    pub path: Option<String>,
    pub external: Option<bool>,
    pub side_effects: Option<bool>,
    pub namespace: Option<String>,
    pub suffix: Option<String>,
    pub plugin_data: Option<u32>,
    pub watch_files: Option<Vec<String>>,
    pub watch_dirs: Option<Vec<String>>,
}

impl_encode_struct!(for OnResolveResponse {
    id,
    plugin_name,
    errors,
    warnings,
    path,
    external,
    side_effects,
    namespace,
    suffix,
    plugin_data,
    watch_files,
    watch_dirs
});

#[derive(Debug, Clone)]
pub struct OnLoadRequest {
    // command: "on-load";
    pub key: u32,
    pub ids: Vec<u32>,
    pub path: String,
    pub namespace: String,
    pub suffix: String,
    pub plugin_data: Option<u32>,
    pub with: IndexMap<String, String>,
}

impl_from_map!(for OnLoadRequest {
    key, ids, path, namespace, suffix, #[optional] plugin_data, with
});

impl_encode_command!(for OnLoadRequest {
  const Command = "on-load";
  key, ids, path, namespace, suffix, plugin_data, with
});

#[derive(Clone, Default)]
pub struct OnLoadResponse {
    pub id: Option<u32>,
    pub plugin_name: Option<String>,
    pub errors: Option<Vec<PartialMessage>>,
    pub warnings: Option<Vec<PartialMessage>>,
    pub contents: Option<Vec<u8>>,
    pub resolve_dir: Option<String>,
    pub loader: Option<String>,
    pub plugin_data: Option<u32>,
    pub watch_files: Option<Vec<String>>,
    pub watch_dirs: Option<Vec<String>>,
}

impl Debug for OnLoadResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnLoadResponse")
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

impl_encode_struct!(for OnLoadResponse {
    id,
    plugin_name,
    errors,
    warnings,
    contents,
    resolve_dir,
    loader,
    plugin_data,
    watch_files,
    watch_dirs
});

#[derive(Debug, Clone)]
pub enum AnyRequest {
    Build(Box<BuildRequest>),
    // Serve(ServeRequest),
    // OnEnd(OnEndRequest),
    // Ping(PingRequest),
    Rebuild(RebuildRequest),
    Dispose(DisposeRequest),
    Cancel(CancelRequest),
    // Watch(WatchRequest),
    // OnServe(OnServeRequest),
    // Transform(TransformRequest),
    // FormatMsgs(FormatMsgsRequest),
    // AnalyzeMetafile(AnalyzeMetafileRequest),
    // OnStart(OnStartRequest),
    // Resolve(ResolveRequest),
    // OnResolve(OnResolveRequest),
    // OnLoad(OnLoadRequest),
}

#[derive(Debug)]
pub enum RequestKind {
    Build(oneshot::Sender<BuildResponse>),
    Dispose(oneshot::Sender<()>),
    Rebuild(oneshot::Sender<RebuildResponse>),
    Cancel(oneshot::Sender<()>),
}

#[derive(Debug, Clone)]
pub enum AnyResponse {
    Build(BuildResponse),
    Serve(ServeResponse),
    OnEnd(OnEndResponse),
    Rebuild(RebuildResponse),
    Transform(TransformResponse),
    FormatMsgs(FormatMsgsResponse),
    AnalyzeMetafile(AnalyzeMetafileResponse),
    OnStart(OnStartResponse),
    Resolve(ResolveResponse),
    OnResolve(OnResolveResponse),
    OnLoad(OnLoadResponse),
}

enum_impl_from!(for AnyResponse {
    Build(BuildResponse),
    Serve(ServeResponse),
    OnEnd(OnEndResponse),
    Rebuild(RebuildResponse),
    Transform(TransformResponse),
    FormatMsgs(FormatMsgsResponse),
    AnalyzeMetafile(AnalyzeMetafileResponse),
    OnStart(OnStartResponse),
    Resolve(ResolveResponse),
    OnResolve(OnResolveResponse),
    OnLoad(OnLoadResponse),
});

#[derive(Debug, Clone)]
pub enum ProtocolMessage {
    Request(AnyRequest),
    Response(AnyResponse),
}

#[derive(Debug, Clone)]
pub struct AnyPacket {
    pub id: u32,
    pub is_request: bool,
    pub value: AnyValue,
}

#[derive(Debug, Clone)]
pub struct ProtocolPacket {
    pub id: u32,
    pub is_request: bool,
    pub value: ProtocolMessage,
}

// impl Decode

#[derive(Debug, Clone)]
pub enum AnyValue {
    Null,
    Bool(bool),
    U32(u32),
    String(String),
    Bytes(Vec<u8>),
    Vec(Vec<AnyValue>),
    Map(IndexMap<String, AnyValue>),
}

impl AnyValue {
    pub fn as_string(&self) -> Result<&String, anyhow::Error> {
        match self {
            AnyValue::String(s) => Ok(s),
            _ => Err(anyhow::anyhow!("Expected string")),
        }
    }

    pub fn as_map(&self) -> Result<&IndexMap<String, AnyValue>, anyhow::Error> {
        match self {
            AnyValue::Map(m) => Ok(m),
            _ => Err(anyhow::anyhow!("Expected map")),
        }
    }
}

impl Decode for bool {
    fn decode_from(buf: &mut Buf) -> Result<Self, anyhow::Error> {
        Ok(buf.read_u8() == 1)
    }
}

impl Decode for u32 {
    fn decode_from(buf: &mut Buf) -> Result<Self, anyhow::Error> {
        Ok(buf.read_u32())
    }
}

impl Decode for String {
    fn decode_from(buf: &mut Buf) -> Result<Self, anyhow::Error> {
        let length = buf.read_u32() as usize;
        let mut string = vec![0; length];
        buf.read_n(length, &mut string);
        String::from_utf8(string).map_err(|e| anyhow::anyhow!("Failed to decode string: {}", e))
    }
}

impl Decode for Vec<u8> {
    fn decode_from(buf: &mut Buf) -> Result<Self, anyhow::Error> {
        let length = buf.read_u32() as usize;
        let mut vec = vec![0; length];
        buf.read_n(length, &mut vec);
        Ok(vec)
    }
}

impl<T: Decode> Decode for Vec<T> {
    fn decode_from(buf: &mut Buf) -> Result<Self, anyhow::Error> {
        let length = buf.read_u32() as usize;
        let mut vec = Vec::with_capacity(length);
        for _ in 0..length {
            vec.push(T::decode_from(buf)?);
        }
        Ok(vec)
    }
}

impl<K: Decode + Hash + Eq, V: Decode> Decode for IndexMap<K, V> {
    fn decode_from(buf: &mut Buf) -> Result<Self, anyhow::Error> {
        let length = buf.read_u32() as usize;
        let mut map = IndexMap::with_capacity(length);
        for _ in 0..length {
            let key = K::decode_from(buf)?;
            let value = V::decode_from(buf)?;
            map.insert(key, value);
        }
        Ok(map)
    }
}

impl Decode for AnyValue {
    fn decode_from(buf: &mut Buf) -> Result<Self, anyhow::Error> {
        let value = buf.read_u8();
        match value {
            0 => Ok(AnyValue::Null),
            1 => Ok(AnyValue::Bool(bool::decode_from(buf)?)),
            2 => Ok(AnyValue::U32(u32::decode_from(buf)?)),
            3 => Ok(AnyValue::String(String::decode_from(buf)?)),
            4 => Ok(AnyValue::Bytes(Vec::decode_from(buf)?)),
            5 => Ok(AnyValue::Vec(Vec::<AnyValue>::decode_from(buf)?)),
            6 => Ok(AnyValue::Map(IndexMap::<String, AnyValue>::decode_from(
                buf,
            )?)),
            _ => Err(anyhow::anyhow!("Invalid value: {}", value)),
        }
    }
}

impl Decode for AnyPacket {
    fn decode_from<'a>(buf: &mut Buf<'a>) -> Result<Self, anyhow::Error> {
        let mut id = u32::decode_from(buf)?;
        let is_request = id & 1 == 0;
        id >>= 1;
        let value = AnyValue::decode_from(buf)?;
        Ok(AnyPacket {
            id,
            is_request,
            value,
        })
    }
}

impl AnyValue {
    pub fn to_type<T: FromAnyValue>(self) -> Result<T, anyhow::Error> {
        T::from_any_value(self)
    }
}

pub fn decode_any_packet(buf: &[u8]) -> Result<AnyPacket, anyhow::Error> {
    let mut buf = Buf::new(buf);
    AnyPacket::decode_from(&mut buf)
}

pub trait FromMap: Sized {
    fn from_map(map: &IndexMap<String, AnyValue>) -> Result<Self, anyhow::Error>;
}

impl FromMap for OnStartRequest {
    fn from_map(map: &IndexMap<String, AnyValue>) -> Result<Self, anyhow::Error> {
        let key = get!(map, "key")?;
        Ok(OnStartRequest { key })
    }
}
