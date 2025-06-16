use esbuild_client::{
    EsbuildFlagsBuilder, Metafile,
    protocol::{BuildRequest, ImportKind},
};
mod common;

use common::{TestDir, create_esbuild_service};

#[tokio::test]
async fn test_basic_build() -> Result<(), Box<dyn std::error::Error>> {
    let test_dir = TestDir::new("esbuild_test_metafile")?;
    let input_file = test_dir.create_file(
        "input.js",
        "import { foo } from './foo.js'; console.log(foo);",
    )?;
    test_dir.create_file("foo.js", "export const foo = 'foo';")?;

    let esbuild = create_esbuild_service(&test_dir).await?;

    let flags = EsbuildFlagsBuilder::default()
        .metafile(true)
        .outfile("output.js".into())
        .bundle(true)
        .build()?
        .to_flags();

    let response = esbuild
        .client()
        .send_build_request(BuildRequest {
            entries: vec![("".into(), input_file.to_string_lossy().into_owned())],
            flags,
            ..Default::default()
        })
        .await?;

    // Check that build succeeded
    assert!(
        response.errors.is_empty(),
        "Build had errors: {:?}",
        response.errors
    );
    assert!(response.output_files.is_some(), "No output files generated");

    assert!(response.metafile.is_some());

    let metafile = serde_json::from_str::<Metafile>(&response.metafile.unwrap()).unwrap();
    assert_eq!(metafile.inputs.len(), 2);
    eprintln!("metafile: {metafile:?}");
    let input = metafile.inputs.get("input.js").unwrap().clone();
    assert!(input.bytes > 0);
    assert_eq!(input.imports.len(), 1);
    assert_eq!(input.imports[0].kind, ImportKind::ImportStatement);

    assert_eq!(metafile.outputs.len(), 1);
    let output = metafile.outputs.get("output.js").unwrap().clone();
    assert!(output.inputs.contains_key("input.js"));

    Ok(())
}
