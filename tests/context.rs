mod common;
use std::sync::{Arc, Mutex};

use anyhow::Error as AnyError;
use common::*;
use esbuild_client::{
    EsbuildFlagsBuilder, OnEndArgs, OnEndResult, OnLoadArgs, OnLoadResult, OnResolveArgs,
    OnResolveResult, OnStartArgs, OnStartResult, PluginHandler,
    protocol::{BuildPlugin, BuildRequest},
};

struct ContextPluginHandler {
    // context: String,
    result: Arc<Mutex<Option<OnEndArgs>>>,
}

#[async_trait::async_trait(?Send)]
impl PluginHandler for ContextPluginHandler {
    async fn on_start(&self, _args: OnStartArgs) -> Result<Option<OnStartResult>, AnyError> {
        Ok(None)
    }

    async fn on_end(&self, args: OnEndArgs) -> Result<Option<OnEndResult>, AnyError> {
        *self.result.lock().unwrap() = Some(args);
        Ok(None)
    }

    async fn on_resolve(&self, _args: OnResolveArgs) -> Result<Option<OnResolveResult>, AnyError> {
        Ok(None)
    }

    async fn on_load(&self, _args: OnLoadArgs) -> Result<Option<OnLoadResult>, AnyError> {
        Ok(None)
    }
}

#[tokio::test]
async fn context_simple() -> Result<(), Box<dyn std::error::Error>> {
    let test_dir = TestDir::new("esbuild_test_context_simple")?;
    let input_file = test_dir.create_file("input.js", "console.log('Hello from esbuild!');")?;
    let output_file = test_dir.path.join("output.js");

    let result = Arc::new(Mutex::new(None));

    let esbuild = create_esbuild_service_with_plugin(
        &test_dir,
        Arc::new(ContextPluginHandler {
            result: result.clone(),
        }),
    )
    .await?;

    let flags = EsbuildFlagsBuilder::default()
        .bundle(true)
        .metafile(true)
        .build()
        .unwrap()
        .to_flags();

    let response = esbuild
        .client()
        .send_build_request(BuildRequest {
            key: 1,
            entries: vec![(
                output_file.to_string_lossy().into_owned(),
                input_file.to_string_lossy().into_owned(),
            )],
            flags,
            context: true,
            plugins: Some(vec![BuildPlugin {
                name: "plugin".to_string(),
                on_end: true,
                on_load: vec![],
                on_resolve: vec![],
                on_start: false,
            }]),
            ..Default::default()
        })
        .await?;

    assert!(response.errors.is_empty());

    let response = esbuild.client().send_rebuild_request(1).await?;
    assert!(response.errors.is_empty());
    assert!(response.warnings.is_empty());

    let result = result.lock().unwrap().take().unwrap();
    assert!(result.output_files.is_some());

    let output_files = result.output_files.unwrap();
    assert!(!output_files.is_empty());

    let output_content = String::from_utf8(output_files[0].contents.clone())?;
    assert!(output_content.contains("Hello from esbuild!"));

    esbuild.client().send_dispose_request(1).await?;

    Ok(())
}
