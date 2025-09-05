use esbuild_client::{EsbuildFlagsBuilder, protocol::BuildRequest};
use pretty_assertions::assert_eq;
mod common;

use common::{TestDir, create_esbuild_service};

#[tokio::test]
async fn test_basic_build() -> Result<(), Box<dyn std::error::Error>> {
    let test_dir = TestDir::new("esbuild_test")?;
    let input_file = test_dir.create_file("input.js", "console.log(process.env.NODE_ENV);")?;

    let esbuild = create_esbuild_service(&test_dir).await?;

    let flags = EsbuildFlagsBuilder::default()
        .bundle(true)
        .minify(false)
        .defines([(
            "process.env.NODE_ENV".to_string(),
            "\"production\"".to_string(),
        )])
        .build_with_defaults();

    let response = esbuild
        .client()
        .send_build_request(BuildRequest {
            entries: vec![("".to_string(), input_file.to_string_lossy().into_owned())],
            write: true,
            flags,
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
    assert!(
        response.write_to_stdout.is_some(),
        "No output files generated"
    );
    let result = String::from_utf8_lossy(response.write_to_stdout.as_deref().unwrap());

    let lines = result.split("\n").collect::<Vec<&str>>();

    assert_eq!(lines[1..].join("\n"), "console.log(\"production\");\n");

    Ok(())
}
