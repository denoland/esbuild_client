use esbuild_rs::{
    EsbuildFlagsBuilder, OnLoadArgs, OnLoadResult, OnResolveArgs, OnResolveResult, OnStartArgs,
    OnStartResult, protocol,
};
use std::{path::Path, sync::Arc};

use anyhow::Error as AnyError;
use async_trait::async_trait;
use esbuild_rs::{EsbuildService, PluginHandler};

pub struct Handler;

#[async_trait(?Send)]
impl PluginHandler for Handler {
    async fn on_resolve(&self, _args: OnResolveArgs) -> Result<Option<OnResolveResult>, AnyError> {
        eprintln!("on-resolve: {:?}", _args);
        Err(anyhow::anyhow!("error"))
        // Ok(None)
        // todo!()
    }

    async fn on_load(&self, _args: OnLoadArgs) -> Result<Option<OnLoadResult>, AnyError> {
        Err(anyhow::anyhow!("error"))
    }

    async fn on_start(&self, _args: OnStartArgs) -> Result<Option<OnStartResult>, AnyError> {
        Ok(None)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    pretty_env_logger::init();
    let path = "/Users/nathanwhit/Library/Caches/esbuild/bin/@esbuild-darwin-arm64@0.25.4";
    let path = Path::new(&path);

    let plugin_handler = Arc::new(Handler);
    let esbuild = EsbuildService::new(path, "0.25.4", plugin_handler.clone())
        .await
        .unwrap();
    let client = esbuild.client().clone();

    {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    res = esbuild.wait_for_exit() => {
                        eprintln!("esbuild exited: {:?}", res);
                        break;
                    }
                }
            }

            Ok::<(), AnyError>(())
        });
    }

    let flags = EsbuildFlagsBuilder::default()
        .outfile("./temp/mod.js".into())
        .build()
        .unwrap();

    let response = client
        .send_build_request(protocol::BuildRequest {
            entries: vec![("".into(), "./main.tsx".into())],
            key: 0,
            flags: flags.to_flags(),
            write: true,
            stdin_contents: None.into(),
            stdin_resolve_dir: None.into(),
            abs_working_dir: "/Users/nathanwhit/Documents/Code/esbuild_rs".into(),
            context: false,
            mangle_cache: None.into(),
            node_paths: vec![],
            plugins: Some(vec![protocol::BuildPlugin {
                name: "test".into(),
                on_start: false,
                on_end: false,
                on_resolve: (vec![protocol::OnResolveSetupOptions {
                    id: 0,
                    filter: ".*".into(),
                    namespace: "".into(),
                }]),
                on_load: vec![protocol::OnLoadSetupOptions {
                    id: 0,
                    filter: ".*".into(),
                    namespace: "".into(),
                }],
            }]),
        })
        .await
        .unwrap();

    eprintln!("build response: {:?}", response);

    if let Some(stdout) = response.write_to_stdout {
        println!("{}", String::from_utf8_lossy(&stdout));
    }
}
