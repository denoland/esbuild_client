use std::path::{Path, PathBuf};
use std::{fs, io};
use sys_traits::{FsFileLock, OpenOptions};

use directories::ProjectDirs;
use esbuild_rs::EsbuildService;
use sys_traits::FsOpen;
use sys_traits::impls::RealSys;

fn base_dir() -> PathBuf {
    let project_dirs =
        ProjectDirs::from("esbuild_rs_test", "esbuild_rs_test", "esbuild_rs_test").unwrap();
    project_dirs.cache_dir().to_path_buf()
}

fn npm_package_name() -> String {
    let platform = match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => "linux-x64".to_string(),
        ("aarch64", "linux") => "linux-arm64".to_string(),
        ("x86_64", "macos" | "apple") => "darwin-x64".to_string(),
        ("aarch64", "macos" | "apple") => "darwin-arm64".to_string(),
        ("x86_64", "windows") => "win32-x64".to_string(),
        ("aarch64", "windows") => "win32-arm64".to_string(),
        _ => panic!(
            "Unsupported platform: {} {}",
            std::env::consts::ARCH,
            std::env::consts::OS
        ),
    };

    format!("@esbuild/{}", platform)
}

pub const ESBUILD_VERSION: &str = "0.25.5";

fn npm_package_url() -> String {
    let package_name = npm_package_name();
    let Some((_, platform)) = package_name.split_once('/') else {
        panic!("Invalid package name: {}", package_name);
    };

    format!(
        "https://registry.npmjs.org/{}/-/{}-{}.tgz",
        package_name, platform, ESBUILD_VERSION
    )
}

struct EsbuildFileLock {
    file: sys_traits::impls::RealFsFile,
}

impl Drop for EsbuildFileLock {
    fn drop(&mut self) {
        let _ = self.file.fs_file_unlock();
    }
}

impl EsbuildFileLock {
    fn new(access_path: &Path) -> Self {
        let path = access_path.parent().unwrap().join(".esbuild.lock");
        let mut options = OpenOptions::new_write();
        options.create = true;
        options.read = true;
        let mut file = RealSys.fs_open(&path, &options).unwrap();
        file.fs_file_lock(sys_traits::FsFileLockMode::Exclusive)
            .unwrap();
        Self { file }
    }
}

pub fn fetch_esbuild() -> PathBuf {
    let esbuild_bin_dir = base_dir().join("bin");
    eprintln!("esbuild_bin_dir: {:?}", esbuild_bin_dir);

    let esbuild_bin_path = esbuild_bin_dir.join("esbuild");
    eprintln!("esbuild_bin_path: {:?}", esbuild_bin_path);
    if esbuild_bin_path.exists() {
        eprintln!("esbuild_bin_path exists");
        return esbuild_bin_path;
    }

    if !esbuild_bin_dir.exists() {
        std::fs::create_dir_all(&esbuild_bin_dir).unwrap();
    }
    let _lock = EsbuildFileLock::new(&esbuild_bin_path);
    if esbuild_bin_path.exists() {
        eprintln!("esbuild_bin_path exists");
        return esbuild_bin_path;
    }

    let esbuild_bin_url = npm_package_url();
    eprintln!("fetching esbuild from: {}", esbuild_bin_url);
    let response = ureq::get(esbuild_bin_url).call().unwrap();

    let reader = response.into_body().into_reader();
    let decoder = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);

    let want_path = if cfg!(target_os = "windows") {
        "package/esbuild.exe"
    } else {
        "package/bin/esbuild"
    };

    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap();

        eprintln!("on entry: {:?}", path);
        if path == std::path::Path::new(want_path) {
            eprintln!("extracting esbuild to: {}", esbuild_bin_path.display());
            std::io::copy(
                &mut entry,
                &mut std::fs::File::create(&esbuild_bin_path).unwrap(),
            )
            .unwrap();

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&esbuild_bin_path, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
            eprintln!("esbuild_bin_path created");

            break;
        }
    }

    esbuild_bin_path
}

pub struct TestDir {
    pub path: PathBuf,
}

impl TestDir {
    pub fn new(name: &str) -> io::Result<Self> {
        let path = std::env::temp_dir().join(name);
        fs::create_dir_all(&path)?;
        Ok(TestDir { path })
    }

    pub fn create_file(&self, name: &str, content: &str) -> io::Result<PathBuf> {
        let file_path = self.path.join(name);
        fs::write(&file_path, content)?;
        Ok(file_path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[allow(dead_code)]
pub async fn create_esbuild_service() -> Result<EsbuildService, Box<dyn std::error::Error>> {
    create_esbuild_service_with_plugin(None).await
}

#[allow(dead_code)]
pub async fn create_esbuild_service_with_plugin(
    plugin_handler: impl esbuild_rs::MakePluginHandler,
) -> Result<EsbuildService, Box<dyn std::error::Error>> {
    let esbuild_path = fetch_esbuild();
    eprintln!("fetched esbuild: {:?}", esbuild_path);
    Ok(EsbuildService::new(esbuild_path, ESBUILD_VERSION, plugin_handler).await?)
}
