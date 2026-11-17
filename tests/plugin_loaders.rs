use std::sync::Arc;

mod common;

use common::{TestDir, create_esbuild_service_with_plugin};
use esbuild_client::{
    BuiltinLoader, EsbuildFlagsBuilder, OnEndArgs, OnEndResult, OnLoadArgs, OnLoadResult,
    OnResolveArgs, OnResolveResult, OnStartArgs, OnStartResult, PluginHandler,
    protocol::BuildRequest,
};

/// A plugin can hand esbuild TSX and CSS module sources. Without a loader the
/// contents are read as plain JavaScript, so both would fail to parse.
#[tokio::test]
async fn test_plugin_loads_tsx_and_local_css() -> Result<(), Box<dyn std::error::Error>> {
    struct VirtualFiles;

    #[async_trait::async_trait(?Send)]
    impl PluginHandler for VirtualFiles {
        async fn on_start(
            &self,
            _args: OnStartArgs,
        ) -> Result<Option<OnStartResult>, esbuild_client::AnyError> {
            Ok(None)
        }

        async fn on_resolve(
            &self,
            args: OnResolveArgs,
        ) -> Result<Option<OnResolveResult>, esbuild_client::AnyError> {
            if args.path.starts_with("virtual:") {
                return Ok(Some(OnResolveResult {
                    path: Some(args.path),
                    namespace: Some("virtual".to_string()),
                    ..Default::default()
                }));
            }
            Ok(None)
        }

        async fn on_load(
            &self,
            args: OnLoadArgs,
        ) -> Result<Option<OnLoadResult>, esbuild_client::AnyError> {
            let (contents, loader) = match args.path.as_str() {
                "virtual:app.tsx" => (
                    r#"
/** @jsx h */
import styles from 'virtual:styles.module.css';

type Props = Record<string, unknown> | null;
const h = (tag: string, props: Props, ...children: unknown[]) => ({ tag, props, children });

export const view: unknown = <div className={styles.box}>hello</div>;
console.log(view);
"#,
                    BuiltinLoader::Tsx,
                ),
                "virtual:styles.module.css" => (".box { color: red; }", BuiltinLoader::LocalCss),
                _ => return Ok(None),
            };
            Ok(Some(OnLoadResult {
                contents: Some(contents.as_bytes().to_vec()),
                loader: Some(loader),
                ..Default::default()
            }))
        }

        async fn on_end(
            &self,
            _args: OnEndArgs,
        ) -> Result<Option<OnEndResult>, esbuild_client::AnyError> {
            Ok(None)
        }
    }

    let test_dir = TestDir::new("esbuild_plugin_loaders_test")?;
    let input_file = test_dir.create_file("main.js", "import 'virtual:app.tsx';")?;
    let output_file = test_dir.path.join("output.js");

    let esbuild = create_esbuild_service_with_plugin(&test_dir, Arc::new(VirtualFiles)).await?;

    let flags = EsbuildFlagsBuilder::default()
        .bundle(true)
        .minify(false)
        .outfile(output_file.to_string_lossy())
        .build();

    let plugin = esbuild_client::protocol::BuildPlugin {
        name: "virtual-files".to_string(),
        on_start: false,
        on_end: false,
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

    let response = esbuild
        .client()
        .send_build_request(BuildRequest {
            entries: vec![(String::new(), input_file.to_string_lossy().into_owned())],
            flags,
            plugins: Some(vec![plugin]),
            ..Default::default()
        })
        .await?
        .unwrap();

    assert!(
        response.errors.is_empty(),
        "Build had errors: {:?}",
        response.errors
    );

    let output_files = response.output_files.expect("output files");
    let javascript = output_files
        .iter()
        .find(|file| file.path.ends_with(".js"))
        .map(|file| String::from_utf8_lossy(&file.contents).into_owned())
        .expect("a JavaScript output");
    assert!(
        javascript.contains("h(\"div\""),
        "the JSX was not compiled: {javascript}"
    );
    assert!(
        !javascript.contains("Record<string"),
        "the types were not removed: {javascript}"
    );

    let stylesheet = output_files
        .iter()
        .find(|file| file.path.ends_with(".css"))
        .map(|file| String::from_utf8_lossy(&file.contents).into_owned())
        .expect("a CSS output");
    assert!(stylesheet.contains("color: red"), "{stylesheet}");
    // A local CSS name is renamed, and the code reads the new name.
    assert!(!stylesheet.contains(".box {"), "{stylesheet}");

    esbuild.stop().await?;
    Ok(())
}
