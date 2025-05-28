mod common;
use common::*;
use esbuild_rs::{EsbuildFlagsBuilder, protocol::BuildRequest};

#[tokio::test]
async fn test_build_with_errors() -> Result<(), Box<dyn std::error::Error>> {
    let test_dir = TestDir::new("esbuild_test_errors")?;
    let input_file = test_dir.create_file("input.js", "console.log('unclosed string")?;
    let output_file = test_dir.path.join("output.js");

    let esbuild = create_esbuild_service().await?;

    let flags = EsbuildFlagsBuilder::default()
        .bundle(true)
        .build()?
        .to_flags();

    let response = esbuild
        .client()
        .send_build_request(BuildRequest {
            entries: vec![(
                output_file.to_string_lossy().into_owned(),
                input_file.to_string_lossy().into_owned(),
            )],
            flags,
            ..Default::default()
        })
        .await?;

    // Check that build failed with errors
    assert!(
        !response.errors.is_empty(),
        "Expected build errors but got none"
    );

    assert_eq!(response.errors.len(), 1);
    assert_eq!(response.errors[0].text, "Unterminated string literal");

    Ok(())
}
