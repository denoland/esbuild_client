use esbuild_client::{EsbuildFlagsBuilder, protocol::BuildRequest};
mod common;

use common::{TestDir, create_esbuild_service};

#[tokio::test]
async fn test_basic_build() -> Result<(), Box<dyn std::error::Error>> {
    let test_dir = TestDir::new("esbuild_test_basic")?;
    let input_file = test_dir.create_file("input.js", "console.log('Hello from esbuild!');")?;
    let output_file = test_dir.path.join("output.js");

    let esbuild = create_esbuild_service(&test_dir).await?;

    let flags = EsbuildFlagsBuilder::default()
        .bundle(true)
        .minify(false)
        .build();

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
        .await?
        .unwrap();

    // Check that build succeeded
    assert!(
        response.errors.is_empty(),
        "Build had errors: {:?}",
        response.errors
    );
    assert!(response.output_files.is_some(), "No output files generated");

    // Check the output files from the response instead of filesystem
    let output_files = response.output_files.unwrap();
    assert!(!output_files.is_empty(), "No output files in response");

    let output_content = String::from_utf8(output_files[0].contents.clone())?;
    assert!(
        output_content.contains("Hello from esbuild"),
        "Output doesn't contain expected content"
    );

    Ok(())
}
