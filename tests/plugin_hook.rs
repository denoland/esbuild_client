mod common;
use common::*;
use esbuild_client::{EsbuildFlagsBuilder, OnEndArgs, OnEndResult, protocol::BuildRequest};
use pretty_assertions::assert_eq;

#[tokio::test]
async fn test_plugin_hook_counting() -> Result<(), Box<dyn std::error::Error>> {
    use async_trait::async_trait;
    use esbuild_client::{
        BuiltinLoader, OnLoadArgs, OnLoadResult, OnResolveArgs, OnResolveResult, OnStartArgs,
        OnStartResult, PluginHandler,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CountingPluginHandler {
        on_start_count: AtomicU32,
        on_resolve_count: AtomicU32,
        on_load_count: AtomicU32,
        on_end_count: AtomicU32,
    }

    impl CountingPluginHandler {
        fn new() -> Self {
            Self {
                on_start_count: AtomicU32::new(0),
                on_resolve_count: AtomicU32::new(0),
                on_load_count: AtomicU32::new(0),
                on_end_count: AtomicU32::new(0),
            }
        }
    }

    #[async_trait(?Send)]
    impl PluginHandler for CountingPluginHandler {
        async fn on_start(
            &self,
            _args: OnStartArgs,
        ) -> Result<Option<OnStartResult>, esbuild_client::AnyError> {
            self.on_start_count.fetch_add(1, Ordering::Relaxed);
            Ok(None)
        }

        async fn on_resolve(
            &self,
            args: OnResolveArgs,
        ) -> Result<Option<OnResolveResult>, esbuild_client::AnyError> {
            self.on_resolve_count.fetch_add(1, Ordering::Relaxed);

            // Only handle our custom virtual files
            if args.path.starts_with("virtual:") {
                let path = args.path.strip_prefix("virtual:").unwrap();
                Ok(Some(OnResolveResult {
                    path: Some(format!("virtual:{}", path)),
                    namespace: Some("virtual".to_string()),
                    ..Default::default()
                }))
            } else {
                Ok(None)
            }
        }

        async fn on_load(
            &self,
            args: OnLoadArgs,
        ) -> Result<Option<OnLoadResult>, esbuild_client::AnyError> {
            self.on_load_count.fetch_add(1, Ordering::Relaxed);

            if args.namespace == "virtual" {
                let content = match args.path.as_str() {
                    "virtual:utils.js" => {
                        r#"
export function add(a, b) {
  return a + b;
}

export function multiply(a, b) {
  return a * b;
}

export const PI = 3.14159;

export default function greet(name) {
  return `Hello, ${name}!`;
}
"#
                    }
                    "virtual:main.js" => {
                        r#"
import greet, { add, PI } from 'virtual:utils.js';

console.log(greet('World'));
console.log('2 + 3 =', add(2, 3));
console.log('PI =', PI);
"#
                    }
                    _ => return Ok(None),
                };

                Ok(Some(OnLoadResult {
                    contents: Some(content.as_bytes().to_vec()),
                    loader: Some(BuiltinLoader::Js),
                    ..Default::default()
                }))
            } else {
                Ok(None)
            }
        }

        async fn on_end(
            &self,
            _args: OnEndArgs,
        ) -> Result<Option<OnEndResult>, esbuild_client::AnyError> {
            self.on_end_count.fetch_add(1, Ordering::Relaxed);
            Ok(Some(OnEndResult {
                errors: None,
                warnings: None,
            }))
        }
    }

    let test_dir = TestDir::new("esbuild_plugin_test")?;
    let input_file = test_dir.create_file("main.js", "import 'virtual:main.js';")?;
    let output_file = test_dir.path.join("output.js");

    let plugin_handler = Arc::new(CountingPluginHandler::new());

    let esbuild = create_esbuild_service_with_plugin(&test_dir, plugin_handler.clone()).await?;

    let mut flags = EsbuildFlagsBuilder::default()
        .bundle(true)
        .minify(false)
        .build()?
        .to_flags();

    // Set up plugin configuration
    let plugin = esbuild_client::protocol::BuildPlugin {
        name: "counting-plugin".to_string(),
        on_start: true,
        on_end: true,
        on_resolve: vec![esbuild_client::protocol::OnResolveSetupOptions {
            id: 1,
            filter: "virtual:.*".to_string(),
            namespace: "".to_string(),
        }],
        on_load: vec![esbuild_client::protocol::OnLoadSetupOptions {
            id: 2,
            filter: ".*".to_string(),
            namespace: "virtual".to_string(),
        }],
    };

    flags.push("--metafile".to_string());

    let response = esbuild
        .client()
        .send_build_request(BuildRequest {
            entries: vec![(
                output_file.to_string_lossy().into_owned(),
                input_file.to_string_lossy().into_owned(),
            )],
            flags,
            plugins: Some(vec![plugin]),
            ..Default::default()
        })
        .await?
        .unwrap();

    // Check that build succeeded
    assert!(
        response.errors.is_empty(),
        "Build had errors: {:?}",
        response.errors
    );

    // Verify hook call counts
    let on_start_calls = plugin_handler.on_start_count.load(Ordering::Relaxed);
    let on_resolve_calls = plugin_handler.on_resolve_count.load(Ordering::Relaxed);
    let on_load_calls = plugin_handler.on_load_count.load(Ordering::Relaxed);
    let on_end_calls = plugin_handler.on_end_count.load(Ordering::Relaxed);

    println!("Hook call counts:");
    println!("  on_start: {}", on_start_calls);
    println!("  on_resolve: {}", on_resolve_calls);
    println!("  on_load: {}", on_load_calls);
    println!("  on_end: {}", on_end_calls);

    // Verify that hooks were called
    assert_eq!(on_start_calls, 1, "on_start should be called at least once");
    assert_eq!(
        on_resolve_calls, 2,
        "on_resolve should be called 2 times for virtual imports"
    );
    assert_eq!(
        on_load_calls, 2,
        "on_load should be called 2 times for virtual files"
    );
    // assert_eq!(on_end_calls, 1, "on_end should be called once");

    // Check that the output contains expected content
    if let Some(output_files) = response.output_files {
        assert!(!output_files.is_empty(), "No output files generated");
        let output_content = String::from_utf8(output_files[0].contents.clone())?;
        assert!(
            output_content.contains("Hello") || output_content.contains("add"),
            "Output should contain content from the virtual modules"
        );
    }

    Ok(())
}
