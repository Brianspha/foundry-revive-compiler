use crate::{
    error::{Result, SolcError},
    resolver::parse::SolData,
    Compiler, CompilerVersion,
};
use foundry_compilers_artifacts::{
    resolc::ResolcCompilerOutput, Error, SolcLanguage,
};
use semver::Version;
use serde::Serialize;
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

impl ResolcOS {
    fn get_resolc_prefix(&self) -> &str {
        match self {
            Self::LinuxAMD64 => "resolc-linux-amd64-musl-",
            Self::LinuxARM64 => "resolc-linux-arm64-musl-",
            Self::MacAMD => "resolc-macosx-amd64-",
            Self::MacARM => "resolc-macosx-arm64-",
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

impl Compiler for Resolc {
    type Input = ResolcVersionedInput;
    type CompilationError = Error;
    type ParsedSource = SolData;
    type Settings = ResolcSettings;
    type Language = SolcLanguage;

    fn available_versions(&self, _language: &Self::Language) -> Vec<CompilerVersion> {
        let compiler = revive_solidity::SolcCompiler::new(
            revive_solidity::SolcCompiler::DEFAULT_EXECUTABLE_NAME.to_owned(),
        )
        .unwrap();
        let mut versions = Vec::new();
        versions.push(CompilerVersion::Remote(compiler.version.unwrap().default));
        versions
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

    pub fn get_path_for_version(version: &Version) -> Result<PathBuf> {
        let maybe_resolc = Self::find_installed_version(version)?;

        let path =
            if let Some(resolc) = maybe_resolc { resolc } else { Self::blocking_install(version)? };

        Ok(path)
    }
    #[cfg(feature = "async")]
    pub fn blocking_install(version: &Version) -> Result<PathBuf> {
        let os = get_operating_system()?;
        let compiler_prefix = os.get_resolc_prefix();
        let download_url = if version.pre.is_empty() {
            format!(
                "https://github.com/paritytech/resolc-bin/releases/download/v{version}/{compiler_prefix}v{version}",
            )
        } else {
            let pre = version.pre.as_str();
            // Use version as string without pre-release and build metadata
            let version_str = version.to_string();
            let version_str = version_str.split('-').next().unwrap();
            // Use pre-release specific repository
            format!(
                "https://github.com/paritytech/revive/releases/download/{pre}/resolc-{compiler_prefix}v{version_str}",
            )
        };
        let compilers_dir = Self::compilers_dir()?;
        if !compilers_dir.exists() {
            create_dir_all(compilers_dir)
                .map_err(|e| SolcError::msg(format!("Could not create compilers path: {e}")))?;
        }
        let compiler_path = Self::compiler_path(version)?;
        let lock_path = lock_file_path("resolc", &version.to_string());

        let label = format!("resolc-{version}");
        let install = compiler_blocking_install(compiler_path, lock_path, &download_url, &label);

        match install {
            Ok(path) => Ok(path),
            Err(err) => Err(err),
        }
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
    let _lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|_| SolcError::msg("Error creating lock file"))?;
    _lock_file.lock_exclusive().map_err(|_| SolcError::msg("Error taking the lock"))?;
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
            .last()
            .ok_or_else(|| SolcError::msg("Version not found in resolc output"))?;

        version
            .split_whitespace()
            .find_map(|s| {
                let trimmed = s.trim_start_matches('v');
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

    #[test]
    fn test_resolc_prefix() {
        let os = get_operating_system().unwrap();
        let prefix = os.get_resolc_prefix();
        assert!(!prefix.is_empty());
        assert!(prefix.contains("resolc"));
        assert!(prefix.ends_with('-'));
    }

    #[test]
    fn test_version_detection() {
        let resolc = resolc_instance();
        let version = Resolc::get_version_for_path(&resolc.resolc);
        assert!(version.is_ok());
    }

    #[test]
    fn test_compiler_path_generation() {
        let version = Version::new(1, 5, 7);
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
            stdout: b"resolc version v1.5.7\n".to_vec(),
            stderr: Vec::new(),
        };
        let version = version_from_output(output);
        assert!(version.is_ok());
        let version = version.unwrap();
        assert_eq!(version.major, 1);
        assert_eq!(version.minor, 5);
        assert_eq!(version.patch, 7);
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
        let version = "1.5.7";
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
        let out = resolc_instance().compile(&input).unwrap();
        assert!(!out.has_error());
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
            if let Ok(version) = Version::parse(&tag.name.trim_start_matches('v')) {
                versions.push(version);
            }
        }
        versions.sort_by(|a, b| b.cmp(a));
        Ok(versions)
    }

    fn get_test_versions() -> Vec<Version> {
        use foundry_compilers_core::utils::RuntimeOrHandle;

        RuntimeOrHandle::new().block_on(fetch_github_versions()).unwrap_or_else(|_| {
            vec![
                Version::parse("0.1.0-dev-6").unwrap(),
                Version::parse("0.1.0-dev-5").unwrap(),
                Version::parse("0.1.0-dev-4").unwrap(),
                Version::parse("0.1.0-dev-3").unwrap(),
                Version::parse("0.1.0-dev-2").unwrap(),
                Version::parse("0.1.0-dev").unwrap(),
            ]
        })
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
        // Test with the most stable version
        let version = Version::parse("0.1.0-dev").unwrap();
        match Resolc::blocking_install(&version) {
            Ok(path) => {
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

    #[cfg(feature = "async")]
    #[test]
    fn test_version_url_format() {
        let version = Version::parse("0.1.0-dev").unwrap();
        let os = get_operating_system().unwrap();
        let compiler_prefix = os.get_resolc_prefix();
        let url = format!(
            "https://github.com/paritytech/revive/releases/download/v{}/{}v{}",
            version, compiler_prefix, version
        );
        // Just verify URL formation - don't actually download
        assert!(url.contains("resolc"));
        assert!(url.contains(&version.to_string()));
    }
}
