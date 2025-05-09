#![allow(dead_code)]
use std::{path::Path, process::Stdio};

pub use anyhow::Error as AnyError;
use indexmap::IndexMap;
use protocol::{AnyPacket, AnyValue, Encode, FromMap, ImportKind, OnStartResponse, ProtocolPacket};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{ChildStdin, ChildStdout},
    sync::{mpsc, oneshot},
};
mod protocol;

pub struct EsbuildService {
    esbuild: tokio::process::Child,
    // channel: EsbuildChannel,
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
            let version = String::from_utf8(message.to_vec()).unwrap();
            eprintln!("version: {}", version);
            state.ready_tx.take().unwrap().send(()).unwrap();
        } else {
            match protocol::decode_any_packet(message) {
                Ok(packet) => {
                    eprintln!("decoded packet: {packet:?}");
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
                eprintln!("got send packet from receiver: {packet:?}");
                let mut encoded = Vec::new();
                packet.encode_into(&mut encoded);
                // eprintln!("encoded: {:?}", encoded);
                stdin.write_all(&encoded).await?;
                eprintln!("wrote packet");
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

impl EsbuildService {
    pub async fn new(
        path: impl AsRef<Path>,
        version: &str,
    ) -> Result<
        (
            Self,
            mpsc::Receiver<AnyPacket>,
            mpsc::Sender<protocol::ProtocolPacket>,
        ),
        AnyError,
    > {
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
        let (packet_tx, packet_rx) = mpsc::channel(100);
        let (response_tx, response_rx) = mpsc::channel(100);
        tokio::spawn(protocol_task(
            stdout,
            stdin,
            ready_tx,
            response_rx,
            packet_tx,
        ));

        let _ = ready_rx.await;

        Ok((Self { esbuild }, packet_rx, response_tx))
    }
}

pub struct OnResolveArgs {
    path: String,
    importer: Option<String>,
    kind: ImportKind,
    namespace: Option<String>,
    resolve_dir: Option<String>,
    with: IndexMap<String, AnyValue>,
}

pub struct OnResolveResult {
    path: String,
    namespace: Option<String>,
}

pub trait PluginInfo {
    fn name(&self) -> &'static str;
    fn provides_on_resolve(&self) -> bool {
        false
    }
}

pub trait Plugin: PluginInfo {
    fn on_resolve(&self, _args: OnResolveArgs) -> Result<Option<OnResolveResult>, AnyError> {
        anyhow::bail!("not implemented")
    }
    // fn on_load(&self)
}

async fn handle_packet(
    packet: AnyPacket,
    response_tx: &mpsc::Sender<protocol::ProtocolPacket>,
) -> Result<(), AnyError> {
    match &packet.value {
        protocol::AnyValue::Map(index_map) => {
            if packet.is_request {
                match index_map.get("command").map(|v| v.as_string()) {
                    Some(Ok(s)) => match s.as_str() {
                        "on-start" => {
                            let on_start = protocol::OnStartRequest::from_map(index_map)?;
                            eprintln!("on-start: {:?}", on_start);
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
                            eprintln!("got on resolve");
                            let on_resolve = protocol::OnResolveRequest::from_map(index_map)?;
                            eprintln!("on-resolve: {:?}", on_resolve);
                            return Ok(());

                            // response_tx
                            //     .send(protocol::ProtocolPacket {
                            //         id: packet.id,
                            //         is_request: false,
                            //     })
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
                todo!("handle: {:?}", packet.value)
            }
        }
        _ => {
            todo!("handle: {:?}", packet.value)
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let path = "/Users/nathanwhit/Library/Caches/esbuild/bin/@esbuild-darwin-arm64@0.25.4";
    let path = Path::new(&path);

    let (mut esbuild, packet_rx, response_tx) = EsbuildService::new(path, "0.25.4").await.unwrap();

    {
        let response_tx = response_tx.clone();
        tokio::spawn(async move {
            let mut packet_rx = packet_rx;
            loop {
                tokio::select! {
                    res = esbuild.esbuild.wait() => {
                        eprintln!("esbuild exited: {:?}", res);
                        std::process::exit(res.unwrap().code().unwrap_or(1));
                    }
                    packet = packet_rx.recv() => {
                        eprintln!("got packet from receiver: {packet:?}");

                        if let Some(packet) = packet {
                            let _ = handle_packet(packet, &response_tx).await.inspect_err(
                                |err| {
                                    eprintln!("failed to handle packet {err}");
                                }
                            );
                        }
                    }
                }
            }

            #[allow(unreachable_code)]
            Ok::<(), AnyError>(())
        });
    }

    let req = protocol::BuildRequest {
        entries: vec![("".into(), "./testing.ts".into())],
        key: 0,
        flags: vec![
            "--color=true".into(),
            "--log-level=warning".into(),
            "--log-limit=0".into(),
            "--format=esm".into(),
            "--platform=node".into(),
            "--tree-shaking=true".into(),
            "--bundle".into(),
            "--outfile=./temp/mod.js".into(),
            "--packages=bundle".into(),
        ],
        write: true,
        stdin_contents: None,
        stdin_resolve_dir: None,
        abs_working_dir: "/Users/nathanwhit/Documents/Code/esbuild-at-home".into(),
        context: false,
        mangle_cache: None,
        node_paths: vec![],
        plugins: Some(vec![
            protocol::BuildPlugin {
                name: "deno".into(),
                on_start: false,
                on_end: false,
                on_resolve: (vec![protocol::OnResolveSetupOptions {
                    id: 0,
                    filter: ".*".into(),
                    namespace: "".into(),
                }]),
                on_load: (vec![protocol::OnLoadSetupOptions {
                    id: 0,
                    filter: ".*".into(),
                    namespace: "".into(),
                }]),
            },
            protocol::BuildPlugin {
                name: "test".into(),
                on_start: false,
                on_end: false,
                on_resolve: (vec![protocol::OnResolveSetupOptions {
                    id: 1,
                    filter: ".*".into(),
                    namespace: "".into(),
                }]),
                on_load: (vec![protocol::OnLoadSetupOptions {
                    id: 2,
                    filter: ".*".into(),
                    namespace: "".into(),
                }]),
            },
        ]),
    };

    let id = 0;
    let packet = protocol::ProtocolPacket {
        id,
        is_request: true,
        value: protocol::ProtocolMessage::Request(protocol::AnyRequest::Build(req)),
    };
    response_tx.send(packet).await.unwrap();
    eprintln!("sending packet");

    eprintln!("sent packet");
    std::future::pending::<()>().await;
    // tokio::time::sleep(std::time::Duration::from_secs(10)).await;
}

#[cfg(test)]
mod test {
    use crate::protocol::Packet;

    use super::*;
    use indexmap::IndexMap;
    use pretty_assertions::assert_eq;
    #[test]
    fn test_decode_packet_build() {
        let input: protocol::Packet<protocol::BuildRequest> = serde_json::from_str(
            r#"{"id":0,"isRequest":true,"value":{"command":"build","key":0,"entries":[["","./testing.ts"]],"flags":["--color=true","--log-level=warning","--log-limit=0","--format=esm","--platform=node","--tree-shaking=true","--bundle","--outfile=./temp/mod.js","--packages=bundle"],"write":true,"stdinContents":null,"stdinResolveDir":null,"absWorkingDir":"/Users/nathanwhit/Documents/Code/esbuild-at-home","nodePaths":[],"context":false,"plugins":[{"name":"deno","onStart":false,"onEnd":false,"onResolve":[{"id":0,"filter":".*","namespace":"deno"}],"onLoad":[{"id":0,"filter":".*","namespace":"deno"}]},{"name":"test","onStart":false,"onEnd":false,"onResolve":[{"id":1,"filter":".*$","namespace":""}],"onLoad":[{"id":2,"filter":".*$","namespace":""}]}]}}"#,
        )
        .unwrap();
        let mut buf = Vec::new();
        input.encode_into(&mut buf);
        assert_eq!(
            buf,
            vec![
                52, 3, 0, 0, 0, 0, 0, 0, 6, 11, 0, 0, 0, 7, 0, 0, 0, 99, 111, 109, 109, 97, 110,
                100, 3, 5, 0, 0, 0, 98, 117, 105, 108, 100, 3, 0, 0, 0, 107, 101, 121, 2, 0, 0, 0,
                0, 7, 0, 0, 0, 101, 110, 116, 114, 105, 101, 115, 5, 1, 0, 0, 0, 5, 2, 0, 0, 0, 3,
                0, 0, 0, 0, 3, 12, 0, 0, 0, 46, 47, 116, 101, 115, 116, 105, 110, 103, 46, 116,
                115, 5, 0, 0, 0, 102, 108, 97, 103, 115, 5, 9, 0, 0, 0, 3, 12, 0, 0, 0, 45, 45, 99,
                111, 108, 111, 114, 61, 116, 114, 117, 101, 3, 19, 0, 0, 0, 45, 45, 108, 111, 103,
                45, 108, 101, 118, 101, 108, 61, 119, 97, 114, 110, 105, 110, 103, 3, 13, 0, 0, 0,
                45, 45, 108, 111, 103, 45, 108, 105, 109, 105, 116, 61, 48, 3, 12, 0, 0, 0, 45, 45,
                102, 111, 114, 109, 97, 116, 61, 101, 115, 109, 3, 15, 0, 0, 0, 45, 45, 112, 108,
                97, 116, 102, 111, 114, 109, 61, 110, 111, 100, 101, 3, 19, 0, 0, 0, 45, 45, 116,
                114, 101, 101, 45, 115, 104, 97, 107, 105, 110, 103, 61, 116, 114, 117, 101, 3, 8,
                0, 0, 0, 45, 45, 98, 117, 110, 100, 108, 101, 3, 23, 0, 0, 0, 45, 45, 111, 117,
                116, 102, 105, 108, 101, 61, 46, 47, 116, 101, 109, 112, 47, 109, 111, 100, 46,
                106, 115, 3, 17, 0, 0, 0, 45, 45, 112, 97, 99, 107, 97, 103, 101, 115, 61, 98, 117,
                110, 100, 108, 101, 5, 0, 0, 0, 119, 114, 105, 116, 101, 1, 1, 13, 0, 0, 0, 115,
                116, 100, 105, 110, 67, 111, 110, 116, 101, 110, 116, 115, 0, 15, 0, 0, 0, 115,
                116, 100, 105, 110, 82, 101, 115, 111, 108, 118, 101, 68, 105, 114, 0, 13, 0, 0, 0,
                97, 98, 115, 87, 111, 114, 107, 105, 110, 103, 68, 105, 114, 3, 48, 0, 0, 0, 47,
                85, 115, 101, 114, 115, 47, 110, 97, 116, 104, 97, 110, 119, 104, 105, 116, 47, 68,
                111, 99, 117, 109, 101, 110, 116, 115, 47, 67, 111, 100, 101, 47, 101, 115, 98,
                117, 105, 108, 100, 45, 97, 116, 45, 104, 111, 109, 101, 9, 0, 0, 0, 110, 111, 100,
                101, 80, 97, 116, 104, 115, 5, 0, 0, 0, 0, 7, 0, 0, 0, 99, 111, 110, 116, 101, 120,
                116, 1, 0, 7, 0, 0, 0, 112, 108, 117, 103, 105, 110, 115, 5, 2, 0, 0, 0, 6, 5, 0,
                0, 0, 4, 0, 0, 0, 110, 97, 109, 101, 3, 4, 0, 0, 0, 100, 101, 110, 111, 7, 0, 0, 0,
                111, 110, 83, 116, 97, 114, 116, 1, 0, 5, 0, 0, 0, 111, 110, 69, 110, 100, 1, 0, 9,
                0, 0, 0, 111, 110, 82, 101, 115, 111, 108, 118, 101, 5, 1, 0, 0, 0, 6, 3, 0, 0, 0,
                2, 0, 0, 0, 105, 100, 2, 0, 0, 0, 0, 6, 0, 0, 0, 102, 105, 108, 116, 101, 114, 3,
                2, 0, 0, 0, 46, 42, 9, 0, 0, 0, 110, 97, 109, 101, 115, 112, 97, 99, 101, 3, 4, 0,
                0, 0, 100, 101, 110, 111, 6, 0, 0, 0, 111, 110, 76, 111, 97, 100, 5, 1, 0, 0, 0, 6,
                3, 0, 0, 0, 2, 0, 0, 0, 105, 100, 2, 0, 0, 0, 0, 6, 0, 0, 0, 102, 105, 108, 116,
                101, 114, 3, 2, 0, 0, 0, 46, 42, 9, 0, 0, 0, 110, 97, 109, 101, 115, 112, 97, 99,
                101, 3, 4, 0, 0, 0, 100, 101, 110, 111, 6, 5, 0, 0, 0, 4, 0, 0, 0, 110, 97, 109,
                101, 3, 4, 0, 0, 0, 116, 101, 115, 116, 7, 0, 0, 0, 111, 110, 83, 116, 97, 114,
                116, 1, 0, 5, 0, 0, 0, 111, 110, 69, 110, 100, 1, 0, 9, 0, 0, 0, 111, 110, 82, 101,
                115, 111, 108, 118, 101, 5, 1, 0, 0, 0, 6, 3, 0, 0, 0, 2, 0, 0, 0, 105, 100, 2, 1,
                0, 0, 0, 6, 0, 0, 0, 102, 105, 108, 116, 101, 114, 3, 3, 0, 0, 0, 46, 42, 36, 9, 0,
                0, 0, 110, 97, 109, 101, 115, 112, 97, 99, 101, 3, 0, 0, 0, 0, 6, 0, 0, 0, 111,
                110, 76, 111, 97, 100, 5, 1, 0, 0, 0, 6, 3, 0, 0, 0, 2, 0, 0, 0, 105, 100, 2, 2, 0,
                0, 0, 6, 0, 0, 0, 102, 105, 108, 116, 101, 114, 3, 3, 0, 0, 0, 46, 42, 36, 9, 0, 0,
                0, 110, 97, 109, 101, 115, 112, 97, 99, 101, 3, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn test_encode_packet() {
        let v = vec![
            ("a".into(), protocol::AnyValue::String("b".into())),
            ("c".into(), protocol::AnyValue::String("d".into())),
            ("e".into(), protocol::AnyValue::Bytes(vec![1u8, 2, 3, 4])),
        ]
        .into_iter()
        .collect::<IndexMap<String, protocol::AnyValue>>();
        let mut buf = Vec::new();
        let packet = Packet {
            id: 1,
            is_request: true,
            value: protocol::AnyValue::Map(v),
        };
        packet.encode_into(&mut buf);

        eprintln!("buf: {:?}", buf);
        // assert_eq!(
        //     buf,
        //     vec![
        //         45, 0, 0, 0, 0, 0, 0, 0, 6, 3, 0, 0, 0, 1, 0, 0, 0, 97, 3, 1, 0, 0, 0, 98, 1, 0, 0, 0,
        //         99, 3, 1, 0, 0, 0, 100, 1, 0, 0, 0, 101, 4, 4, 0, 0, 0, 1, 2, 3, 4
        //     ]
        // );
        assert_eq!(
            buf,
            vec![
                45, 0, 0, 0, 2, 0, 0, 0, 6, 3, 0, 0, 0, 1, 0, 0, 0, 97, 3, 1, 0, 0, 0, 98, 1, 0, 0,
                0, 99, 3, 1, 0, 0, 0, 100, 1, 0, 0, 0, 101, 4, 4, 0, 0, 0, 1, 2, 3, 4
            ]
        );
    }
}
