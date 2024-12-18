use crate::{
    error::{Result, SolcError},
    resolver::parse::SolData,
    Compiler, CompilerVersion,
};
use foundry_compilers_artifacts::{resolc::ResolcCompilerOutput, Error, SolcLanguage};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    str::FromStr,
};

#[cfg(feature = "async")]
use std::{
    fs::{self, create_dir_all, set_permissions, File},
    io::Write,
};

#[cfg(target_family = "unix")]
#[cfg(feature = "async")]
use std::os::unix::fs::PermissionsExt;

use super::{ResolcInput, ResolcSettings, ResolcVersionedInput};

#[derive(Debug, Clone, Serialize)]
enum ResolcOS {
    LinuxAMD64,
    LinuxARM64,
    MacAMD,
    MacARM,
}

fn get_operating_system() -> Result<ResolcOS> {
    match std::env::consts::OS {
        "linux" => match std::env::consts::ARCH {
            "aarch64" => Ok(ResolcOS::LinuxARM64),
            _ => Ok(ResolcOS::LinuxAMD64),
        },
        "macos" | "darwin" => match std::env::consts::ARCH {
            "aarch64" => Ok(ResolcOS::MacARM),
            _ => Ok(ResolcOS::MacAMD),
        },
        _ => Err(SolcError::msg(format!("Unsupported operating system {}", std::env::consts::OS))),
    }
}
impl Default for ResolcOS {
    fn default() -> Self {
        Self::MacARM
    }
}
impl ResolcOS {
    fn get_resolc_prefix(&self) -> &str {
        match self {
            Self::LinuxAMD64 => "resolc",
            Self::LinuxARM64 => "resolc",
            Self::MacAMD => "resolc",
            Self::MacARM => "resolc",
        }
    }
    fn get_solc_prefix(&self) -> &str {
        match self {
            Self::LinuxAMD64 => "solc-linux-amd64-",
            Self::LinuxARM64 => "solc-linux-arm64-",
            Self::MacAMD => "solc-macosx-amd64-",
            Self::MacARM => "solc-macosx-arm64-",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Resolc {
    pub resolc: PathBuf,
    pub extra_args: Vec<String>,
    pub base_path: Option<PathBuf>,
    pub allow_paths: BTreeSet<PathBuf>,
    pub include_paths: BTreeSet<PathBuf>,
}
#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SolcVersionInfo {
    /// The solc compiler version (e.g: 0.8.20)
    pub version: Version,
    /// The full revive solc compiler version (e.g: 0.8.20-1.0.1)
    pub revive_version: Option<Version>,
}
impl Compiler for Resolc {
    type Input = ResolcVersionedInput;
    type CompilationError = Error;
    type ParsedSource = SolData;
    type Settings = ResolcSettings;
    type Language = SolcLanguage;

    /// Instead of using specific sols version we are going to autodetect
    /// Installed versions
    fn available_versions(&self, _language: &Self::Language) -> Vec<CompilerVersion> {
        let mut all_versions = Resolc::solc_installed_versions()
            .into_iter()
            .map(CompilerVersion::Installed)
            .collect::<Vec<_>>();
        let mut uniques = all_versions
            .iter()
            .map(|v| {
                let v = v.as_ref();
                (v.major, v.minor, v.patch)
            })
            .collect::<std::collections::HashSet<_>>();
        all_versions.extend(
            Resolc::solc_available_versions()
                .into_iter()
                .filter(|v| uniques.insert((v.major, v.minor, v.patch)))
                .map(CompilerVersion::Remote),
        );
        all_versions.sort_unstable();
        all_versions
    }

    fn compile(
        &self,
        _input: &Self::Input,
    ) -> Result<crate::compilers::CompilerOutput<Error>, SolcError> {
        todo!("Implement if needed");
    }
}

impl Resolc {
    pub fn new(path: PathBuf) -> Result<Self> {
        Ok(Self {
            resolc: path,
            extra_args: Vec::new(),
            base_path: None,
            allow_paths: Default::default(),
            include_paths: Default::default(),
        })
    }
    pub fn solc_available_versions() -> Vec<Version> {
        let mut ret = vec![];
        let min_max_patch_by_minor_versions =
            vec![(4, 12, 26), (5, 0, 17), (6, 0, 12), (7, 0, 6), (8, 0, 28)];
        for (minor, min_patch, max_patch) in min_max_patch_by_minor_versions {
            for i in min_patch..=max_patch {
                ret.push(Version::new(0, minor, i));
            }
        }

        ret
    }
    pub fn get_solc_version_info(path: &Path) -> Result<SolcVersionInfo, SolcError> {
        let mut cmd = Command::new(path);
        cmd.arg("--version").stdin(Stdio::piped()).stderr(Stdio::piped()).stdout(Stdio::piped());
        debug!(?cmd, "getting solc versions");

        let output = cmd.output().map_err(|e| SolcError::io(e, path))?;
        trace!(?output);

        if !output.status.success() {
            return Err(SolcError::solc_output(&output));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();

        let version =
            lines.get(1).ok_or_else(|| SolcError::msg("Version not found in Solc output"))?;
        let version =
            Version::from_str(&version.trim_start_matches("Version: ").replace(".g++", ".gcc"))?;

        let revive_version = lines.last().and_then(|line| {
            if line.starts_with("Revive") {
                let version_str = line.trim_start_matches("Revive:").trim();
                Version::parse(version_str).ok()
            } else {
                None
            }
        });

        Ok(SolcVersionInfo { version, revive_version })
    }
    pub fn solc_installed_versions() -> Vec<Version> {
        if let Ok(dir) = Self::compilers_dir() {
            let os = get_operating_system().unwrap();
            let solc_prefix = os.get_solc_prefix();
            let mut versions: Vec<Version> = walkdir::WalkDir::new(dir)
                .max_depth(1)
                .into_iter()
                .filter_map(std::result::Result::ok)
                .filter(|e| e.file_type().is_file())
                .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
                .filter_map(|e| {
                    e.strip_prefix(solc_prefix)
                        .and_then(|s| s.split('-').next())
                        .and_then(|s| Version::parse(s).ok())
                })
                .collect();
            versions.sort();
            versions
        } else {
            vec![]
        }
    }
    pub fn get_path_for_version(version: &Version) -> Result<PathBuf> {
        let maybe_resolc = Self::find_installed_version(version)?;

        let path =
            if let Some(resolc) = maybe_resolc { resolc } else { Self::blocking_install(version)? };

        Ok(path)
    }
    #[cfg(feature = "async")]
    pub fn blocking_install(version: &Version) -> Result<PathBuf> {
        let os: ResolcOS = get_operating_system()?;
        let compiler_prefix = os.get_resolc_prefix();

        let download_url = format!(
            "https://github.com/paritytech/revive/releases/download/v{version}/{compiler_prefix}"
        );

        let compilers_dir = Self::compilers_dir()?;
        if !compilers_dir.exists() {
            create_dir_all(compilers_dir)
                .map_err(|e| SolcError::msg(format!("Could not create compilers path: {e}")))?;
        }

        let compiler_path = Self::compiler_path(version)?;
        let lock_path = lock_file_path("resolc", &version.to_string());
        let label = format!("resolc-{version}");

        compiler_blocking_install(compiler_path, lock_path, &download_url, &label)
    }
    pub fn get_version_for_path(path: &Path) -> Result<Version> {
        let mut cmd = Command::new(path);
        cmd.arg("--version").stdin(Stdio::piped()).stderr(Stdio::piped()).stdout(Stdio::piped());
        debug!(?cmd, "getting Resolc version");
        let output = cmd.output().map_err(map_io_err(path))?;
        trace!(?output);
        let version = version_from_output(output)?;
        debug!(%version);
        Ok(version)
    }

    fn compilers_dir() -> Result<PathBuf> {
        let mut compilers_dir =
            dirs::home_dir().ok_or(SolcError::msg("Could not build Resolc - homedir not found"))?;
        compilers_dir.push(".revive");
        Ok(compilers_dir)
    }

    fn compiler_path(version: &Version) -> Result<PathBuf> {
        let os = get_operating_system()?;
        Ok(Self::compilers_dir()?.join(format!("{}v{}", os.get_resolc_prefix(), version)))
    }

    pub fn find_installed_version(version: &Version) -> Result<Option<PathBuf>> {
        let resolc = Self::compiler_path(version)?;

        if !resolc.is_file() {
            return Ok(None);
        }
        Ok(Some(resolc))
    }

    pub fn compile(&self, input: &ResolcInput) -> Result<ResolcCompilerOutput> {
        match self.compile_output::<ResolcInput>(input) {
            Ok(results) => {
                let output = std::str::from_utf8(&results).map_err(|_| SolcError::InvalidUtf8)?;
                serde_json::from_str(output).map_err(|e| SolcError::msg(e.to_string()))
            }
            Err(_) => Ok(ResolcCompilerOutput::default()),
        }
    }

    pub fn compile_output<T: Serialize>(&self, input: &ResolcInput) -> Result<Vec<u8>> {
        let mut cmd = self.configure_cmd();
        let mut child = cmd.spawn().map_err(|err| SolcError::io(err, &self.resolc))?;

        let stdin = child.stdin.as_mut().unwrap();
        serde_json::to_writer(stdin, input)?;

        let output = child.wait_with_output().map_err(|err| SolcError::io(err, &self.resolc))?;

        compile_output(output)
    }

    fn configure_cmd(&self) -> Command {
        let mut cmd = Command::new(&self.resolc);
        cmd.stdin(Stdio::piped()).stderr(Stdio::piped()).stdout(Stdio::piped());
        cmd.args(&self.extra_args);
        cmd.arg("--standard-json");
        cmd
    }
}

#[cfg(feature = "async")]
fn compiler_blocking_install(
    compiler_path: PathBuf,
    lock_path: PathBuf,
    download_url: &str,
    label: &str,
) -> Result<PathBuf> {
    use foundry_compilers_core::utils::RuntimeOrHandle;
    trace!("blocking installing {label}");
    RuntimeOrHandle::new().block_on(async {
        let client = reqwest::Client::new();
        let response = client
            .get(download_url)
            .send()
            .await
            .map_err(|e| SolcError::msg(format!("Failed to download {label} file: {e}")))?;

        if response.status().is_success() {
            let content = response
                .bytes()
                .await
                .map_err(|e| SolcError::msg(format!("failed to download {label} file: {e}")))?;
            trace!("downloaded {label}");

            trace!("try to get lock for {label}");
            let _lock = try_lock_file(lock_path)?;
            trace!("got lock for {label}");

            if !compiler_path.exists() {
                trace!("creating binary for {label}");
                let mut output_file = File::create(&compiler_path).map_err(|e| {
                    SolcError::msg(format!("Failed to create output {label} file: {e}"))
                })?;

                output_file.write_all(&content).map_err(|e| {
                    SolcError::msg(format!("Failed to write the downloaded {label} file: {e}"))
                })?;

                set_permissions(&compiler_path, PermissionsExt::from_mode(0o755)).map_err(|e| {
                    SolcError::msg(format!("Failed to set {label} permissions: {e}"))
                })?;
            } else {
                trace!("found binary for {label}");
            }
        } else {
            return Err(SolcError::msg(format!(
                "Failed to download {label} file: status code {}",
                response.status()
            )));
        }
        trace!("{label} installation completed");
        Ok(compiler_path)
    })
}

#[cfg(feature = "async")]
fn try_lock_file(lock_path: PathBuf) -> Result<LockFile> {
    use fs4::FileExt;

    println!("Attempting to create lock file at: {:?}", lock_path);
    if let Some(parent) = lock_path.parent() {
        if !parent.exists() {
            println!("Parent directory does not exist: {:?}", parent);
            std::fs::create_dir_all(parent)
                .map_err(|e| SolcError::msg(format!("Failed to create parent directory: {}", e)))?;
        }
    }

    let _lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| SolcError::msg(format!("Error creating lock file: {}", e)))?;

    _lock_file
        .lock_exclusive()
        .map_err(|e| SolcError::msg(format!("Error taking the lock: {}", e)))?;

    Ok(LockFile { lock_path, _lock_file })
}

#[cfg(feature = "async")]
struct LockFile {
    _lock_file: File,
    lock_path: PathBuf,
}

#[cfg(feature = "async")]
impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

#[cfg(feature = "async")]
fn lock_file_path(compiler: &str, version: &str) -> PathBuf {
    Resolc::compilers_dir()
        .expect("could not detect resolc compilers directory")
        .join(format!(".lock-{compiler}-{version}"))
}

fn map_io_err(resolc_path: &Path) -> impl FnOnce(std::io::Error) -> SolcError + '_ {
    move |err| SolcError::io(err, resolc_path)
}

fn version_from_output(output: Output) -> Result<Version> {
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let version = stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .find(|l| l.contains("version"))
            .ok_or_else(|| SolcError::msg("Version not found in resolc output"))?;

        version
            .split_whitespace()
            .find(|s| s.starts_with("0.") || s.starts_with("v0."))
            .and_then(|s| {
                let trimmed = s.trim_start_matches('v').split('+').next().unwrap_or(s);
                Version::from_str(trimmed).ok()
            })
            .ok_or_else(|| SolcError::msg("Unable to retrieve version from resolc output"))
    } else {
        Err(SolcError::solc_output(&output))
    }
}

fn compile_output(output: Output) -> Result<Vec<u8>> {
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(SolcError::solc_output(&output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;
    use std::os::unix::process::ExitStatusExt;
    use tempfile::tempdir;

    #[derive(Debug, Deserialize)]
    struct GitHubTag {
        name: String,
    }

    fn resolc_instance() -> Resolc {
        Resolc::new(PathBuf::from(
            revive_solidity::SolcCompiler::DEFAULT_EXECUTABLE_NAME.to_owned(),
        ))
        .unwrap()
    }

    #[test]
    fn test_get_operating_system() {
        let os = get_operating_system().unwrap();
        match std::env::consts::OS {
            "linux" => match std::env::consts::ARCH {
                "aarch64" => assert!(matches!(os, ResolcOS::LinuxARM64)),
                _ => assert!(matches!(os, ResolcOS::LinuxAMD64)),
            },
            "macos" | "darwin" => match std::env::consts::ARCH {
                "aarch64" => assert!(matches!(os, ResolcOS::MacARM)),
                _ => assert!(matches!(os, ResolcOS::MacAMD)),
            },
            _ => panic!("Unsupported OS for test"),
        }
    }

    #[cfg(feature = "async")]
    #[test]
    fn test_install_and_verify_version() {
        use std::process::Command;

        let expected_version = Version::parse("0.1.0-dev.6").unwrap();

        let installed_path = match Resolc::blocking_install(&expected_version) {
            Ok(path) => path,
            Err(e) => {
                panic!("Failed to install version {}: {}", expected_version, e);
            }
        };

        assert!(installed_path.exists(), "Installed binary should exist");
        assert!(installed_path.is_file(), "Should be a file");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&installed_path).unwrap();
            let permissions = metadata.permissions();
            assert!(permissions.mode() & 0o111 != 0, "Binary should be executable");
        }

        let version_output = Command::new(&installed_path).arg("--version").output();

        match version_output {
            Ok(output) => {
                println!("Direct execution output: {:?}", String::from_utf8_lossy(&output.stdout));
                println!("Direct execution stderr: {:?}", String::from_utf8_lossy(&output.stderr));
            }
            Err(e) => {
                println!("Direct execution error: {}", e);
            }
        }

        match Resolc::get_version_for_path(&installed_path) {
            Ok(actual_version) => {
                assert_eq!(
                    actual_version, expected_version,
                    "Installed version should match requested version"
                );
            }
            Err(e) => {
                println!("Error getting version: {}", e);
                println!("Installed path: {:?}", installed_path);

                #[cfg(unix)]
                {
                    let file_type = Command::new("file")
                        .arg(&installed_path)
                        .output()
                        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                        .unwrap_or_else(|e| format!("Failed to run 'file': {}", e));
                    println!("File type: {}", file_type);
                }

                panic!("Failed to get version from installed binary with detailed error: {}", e);
            }
        }

        match Resolc::find_installed_version(&expected_version) {
            Ok(Some(found_path)) => {
                assert_eq!(found_path, installed_path, "Found path should match installed path");
            }
            Ok(None) => {
                panic!("Version {} not found after installation", expected_version);
            }
            Err(e) => {
                panic!("Error finding installed version: {}", e);
            }
        }
    }

    #[test]
    fn test_resolc_prefix() {
        let os = get_operating_system().unwrap();
        let prefix = os.get_resolc_prefix();
        assert!(!prefix.is_empty());
        assert!(prefix.contains("resolc"));
    }

    #[test]
    fn test_compiler_path_generation() {
        let version = Version::new(0, 1, 0);
        let path = Resolc::compiler_path(&version);
        assert!(path.is_ok());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains(&version.to_string()));
    }

    #[test]
    fn test_compilers_dir_creation() {
        let dir = Resolc::compilers_dir();
        assert!(dir.is_ok());
        let dir_path = dir.unwrap();
        assert!(dir_path.ends_with(".revive"));
    }
    #[cfg(feature = "async")]
    #[test]
    fn test_find_installed_versions() {
        let versions: Vec<_> = get_test_versions().into_iter().take(2).collect();

        for version in &versions {
            match Resolc::blocking_install(version) {
                Ok(path) => {
                    let result = Resolc::find_installed_version(version);
                    assert!(result.is_ok());
                    let path_opt = result.unwrap();
                    assert!(path_opt.is_some());
                    assert_eq!(path_opt.unwrap(), path);
                }
                Err(e) => {
                    println!("Warning: Failed to install version {}: {}", version, e);
                    continue;
                }
            }
        }
    }

    #[cfg(feature = "async")]
    #[test]
    fn test_install_single_version() {
        let version = Version::parse("0.1.0-dev.6").unwrap();
        match Resolc::blocking_install(&version) {
            Ok(path) => {
                println!("version: {:?}", version);
                assert!(path.exists(), "Path should exist for version {}", version);
                assert!(path.is_file(), "Should be a file for version {}", version);
            }
            Err(e) => {
                println!("Warning: Failed to install version {}: {}", version, e);
            }
        }
    }

    #[cfg(feature = "async")]
    #[test]
    fn test_find_nonexistent_version() {
        let version = Version::parse("99.99.99-dev").unwrap();
        let result = Resolc::find_installed_version(&version);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_new_resolc_instance() {
        let path = PathBuf::from("test_resolc");
        let resolc = Resolc::new(path.clone());
        assert!(resolc.is_ok());
        let resolc = resolc.unwrap();
        assert_eq!(resolc.resolc, path);
        assert!(resolc.extra_args.is_empty());
        assert!(resolc.base_path.is_none());
        assert!(resolc.allow_paths.is_empty());
        assert!(resolc.include_paths.is_empty());
    }

    #[test]
    fn test_version_parsing() {
        let output = Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: b"resolc version v0.1.0\n".to_vec(),
            stderr: Vec::new(),
        };
        let version = version_from_output(output);
        assert!(version.is_ok());
        let version = version.unwrap();
        assert_eq!(version.major, 0);
        assert_eq!(version.minor, 1);
        assert_eq!(version.patch, 0);
    }

    #[test]
    fn test_failed_version_parsing() {
        let output = Output {
            status: std::process::ExitStatus::from_raw(1),
            stdout: Vec::new(),
            stderr: b"error\n".to_vec(),
        };
        let version = version_from_output(output);
        assert!(version.is_err());
    }

    #[test]
    fn test_invalid_version_output() {
        let output = Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: b"invalid version format\n".to_vec(),
            stderr: Vec::new(),
        };
        let version = version_from_output(output);
        assert!(version.is_err());
    }

    #[cfg(feature = "async")]
    #[test]
    fn test_lock_file_path() {
        let version = "0.1.0";
        let lock_path = lock_file_path("resolc", version);
        assert!(lock_path.to_string_lossy().contains("resolc"));
        assert!(lock_path.to_string_lossy().contains(version));
        assert!(lock_path.to_string_lossy().contains(".lock"));
    }

    #[test]
    fn test_configure_cmd() {
        let resolc = resolc_instance();
        let cmd = resolc.configure_cmd();
        assert!(cmd.get_args().any(|arg| arg == "--standard-json"));
    }

    #[test]
    fn test_compile_empty_input() {
        let resolc = resolc_instance();
        let input = ResolcInput::default();
        let result = resolc.compile(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compile_output_success() {
        let output = Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: b"test output".to_vec(),
            stderr: Vec::new(),
        };
        let result = compile_output(output);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"test output");
    }

    #[test]
    fn test_compile_output_failure() {
        let output = Output {
            status: std::process::ExitStatus::from_raw(1),
            stdout: Vec::new(),
            stderr: b"error".to_vec(),
        };
        let result = compile_output(output);
        assert!(result.is_err());
    }

    #[test]
    fn resolc_compile_works() {
        let input = include_str!("../../../../../test-data/resolc/input/compile-input.json");
        let input: ResolcInput = serde_json::from_str(input).unwrap();
        let out: ResolcCompilerOutput = resolc_instance().compile(&input).unwrap();
        assert!(!out.has_error());
    }
    fn normalize_version(version_str: &str) -> Result<Version, semver::Error> {
        let normalized = version_str.replace("dev-", "dev.");
        Version::parse(&normalized)
    }
    async fn fetch_github_versions() -> Result<Vec<Version>> {
        let client = reqwest::Client::new();
        let tags: Vec<GitHubTag> = client
            .get("https://api.github.com/repos/paritytech/revive/tags")
            .header("User-Agent", "revive-test")
            .send()
            .await
            .map_err(|e| SolcError::msg(format!("Failed to fetch tags: {}", e)))?
            .json()
            .await
            .map_err(|e| SolcError::msg(format!("Failed to parse tags: {}", e)))?;

        let mut versions = Vec::new();
        for tag in tags {
            if let Ok(version) = normalize_version(&tag.name.trim_start_matches('v')) {
                versions.push(version);
            }
        }
        versions.sort_by(|a, b| b.cmp(a));
        Ok(versions)
    }

    fn get_test_versions() -> Vec<Version> {
        use foundry_compilers_core::utils::RuntimeOrHandle;

        RuntimeOrHandle::new()
            .block_on(fetch_github_versions())
            .unwrap_or_else(|_| vec![Version::parse("0.1.0-dev-6").unwrap()])
    }

    #[cfg(feature = "async")]
    mod install_tests {
        use super::*;

        fn setup_test_paths(version: &str) -> (PathBuf, PathBuf) {
            let temp_dir = tempdir().unwrap();
            let compiler_path = temp_dir.path().join(format!("resolc-{}", version));
            let lock_path = temp_dir.path().join(format!(".lock-resolc-{}", version));
            (compiler_path, lock_path)
        }

        #[test]
        fn test_compiler_blocking_install_dev() {
            let version = "0.1.0-dev";
            let (compiler_path, lock_path) = setup_test_paths(version);
            let url = format!(
                "https://github.com/paritytech/revive/releases/download/v{version}/resolc",
            );
            let label = format!("resolc-{version}");

            let result = compiler_blocking_install(compiler_path, lock_path, &url, &label);
            println!("result: {:?}", result);
            assert!(!result.is_err());
        }

        #[test]
        fn test_compiler_blocking_install_invalid_url() {
            let (compiler_path, lock_path) = setup_test_paths("test");
            let result = compiler_blocking_install(
                compiler_path,
                lock_path,
                "https://invalid.url/not-found",
                "test",
            );
            assert!(result.is_err());
        }

        #[test]
        fn test_compiler_blocking_install_existing_file() {
            let version = "0.1.0-dev.6";
            let (compiler_path, lock_path) = setup_test_paths(version);

            let os: ResolcOS = get_operating_system().unwrap_or_default();
            let compiler_prefix = os.get_resolc_prefix();

            std::fs::create_dir_all(compiler_path.parent().unwrap())
                .expect("Failed to create parent directory");

            std::fs::write(&compiler_path, "test").unwrap();

            let url = format!(
                "https://github.com/paritytech/revive/releases/download/v{version}/{compiler_prefix}",
            );
            let label = format!("resolc-{version}");

            let result = compiler_blocking_install(compiler_path.clone(), lock_path, &url, &label);

            assert!(!result.is_err());
            assert!(compiler_path.exists());
        }
    }

    #[test]
    fn test_version_with_whitespace() {
        let output = Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: b"resolc version   v0.1.0  \n".to_vec(),
            stderr: Vec::new(),
        };
        let version = version_from_output(output);
        assert!(version.is_ok());
        let version = version.unwrap();
        assert_eq!(version.to_string(), "0.1.0");
    }

    #[test]
    fn test_version_with_extra_info() {
        let output = Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: b"Some other info\nresolc version v0.1.0\nExtra info".to_vec(),
            stderr: Vec::new(),
        };
        let version = version_from_output(output);
        assert!(version.is_ok());
        let version = version.unwrap();
        assert_eq!(version.to_string(), "0.1.0");
    }

    #[test]
    fn test_compile_output_with_stderr() {
        let output = Output {
            status: std::process::ExitStatus::from_raw(1),
            stdout: Vec::new(),
            stderr: b"compilation error\n".to_vec(),
        };
        let result = compile_output(output);
        assert!(result.is_err());
        assert!(format!("{:?}", result.unwrap_err()).contains("compilation error"));
    }

    #[test]
    fn test_solc_available_versions_sorted() {
        let versions = Resolc::solc_available_versions();
        let mut sorted = versions.clone();
        sorted.sort();
        assert_eq!(versions, sorted, "Versions should be returned in sorted order");

        for version in versions {
            assert_eq!(version.major, 0, "Major version should be 0");
            assert!(
                version.minor >= 4 && version.minor <= 8,
                "Minor version should be between 4 and 8"
            );
        }
    }

    #[test]
    fn test_compiler_path_with_spaces() {
        let version = Version::new(0, 1, 0);
        let path = Resolc::compiler_path(&version).unwrap();
        assert!(!path.to_string_lossy().contains(" "));
    }
}
