use std::fs;
use std::path::Path;

use esbuild_rs::{EsbuildFlagsBuilder, EsbuildService, protocol::BuildRequest};
mod common;

use common::{ESBUILD_VERSION, fetch_esbuild};

#[tokio::test]
async fn test_basic_build() -> Result<(), Box<dyn std::error::Error>> {
    let esbuild_path = fetch_esbuild();

    // Create a simple test input file
    let temp_dir = std::env::temp_dir().join("esbuild_test");
    fs::create_dir_all(&temp_dir)?;

    let input_file = temp_dir.join("input.js");
    fs::write(&input_file, "console.log('Hello from esbuild!');")?;

    let output_file = temp_dir.join("output.js");

    // Create esbuild service with no plugin handler
    let esbuild = EsbuildService::new(esbuild_path, ESBUILD_VERSION, None).await?;

    let flags = EsbuildFlagsBuilder::default()
        .bundle(true)
        .minify(false)
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

    // Check that build succeeded
    assert!(
        response.errors.is_empty(),
        "Build had errors: {:?}",
        response.errors
    );
    assert!(response.output_files.is_some(), "No output files generated");

    // Verify output file exists and has content
    assert!(output_file.exists(), "Output file was not created");
    let output_content = fs::read_to_string(&output_file)?;
    assert!(
        output_content.contains("Hello from esbuild"),
        "Output doesn't contain expected content"
    );

    // Cleanup
    fs::remove_dir_all(&temp_dir)?;

    Ok(())
}

#[tokio::test]
async fn test_build_with_errors() -> Result<(), Box<dyn std::error::Error>> {
    let esbuild_path = fetch_esbuild();

    // Create a test input file with syntax errors
    let temp_dir = std::env::temp_dir().join("esbuild_test_errors");
    fs::create_dir_all(&temp_dir)?;

    let input_file = temp_dir.join("input.js");
    fs::write(&input_file, "console.log('unclosed string")?; // Missing closing quote

    let output_file = temp_dir.join("output.js");

    let esbuild = EsbuildService::new(esbuild_path, ESBUILD_VERSION, None).await?;

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

    // Cleanup
    fs::remove_dir_all(&temp_dir)?;

    Ok(())
}
