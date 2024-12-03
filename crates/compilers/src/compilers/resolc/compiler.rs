use foundry_compilers_artifacts::{CompilerOutput, Error, SolcLanguage};
use foundry_compilers_core::error::{Result, SolcError};
use semver::Version;
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    str::FromStr,
};

use crate::{compilers, resolver::parse::SolData, Compiler, CompilerVersion};

use super::{ResolcInput, ResolcSettings, ResolcVersionedInput};

#[derive(Clone, Debug)]
pub struct Resolc {
    pub resolc: PathBuf,
    pub extra_args: Vec<String>,
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
    ) -> Result<
        compilers::CompilerOutput<foundry_compilers_artifacts::Error>,
        foundry_compilers_core::error::SolcError,
    > {
        todo!("Implement if needed");
    }
}

impl Resolc {
    pub fn new(path: PathBuf) -> Result<Self> {
        Ok(Self { resolc: path, extra_args: Vec::new() })
    }

    pub fn compile(&self, input: &ResolcInput) -> Result<CompilerOutput> {
        match self.compile_output::<ResolcInput>(input) {
            Ok(results) => {
                let output = std::str::from_utf8(&results).map_err(|_| SolcError::InvalidUtf8)?;
                serde_json::from_str(output).map_err(|e| SolcError::msg(e.to_string()))
            }
            Err(_) => Ok(CompilerOutput::default()),
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
}

fn map_io_err(path: &Path) -> impl FnOnce(std::io::Error) -> SolcError + '_ {
    move |err| SolcError::io(err, path)
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
mod test {
    use super::*;

    fn resolc_instance() -> Resolc {
        Resolc::new(PathBuf::from(
            revive_solidity::SolcCompiler::DEFAULT_EXECUTABLE_NAME.to_owned(),
        ))
        .unwrap()
    }

    #[test]
    fn resolc_version_works() {
        Resolc::get_version_for_path(&mut PathBuf::from(
            revive_solidity::SolcCompiler::DEFAULT_EXECUTABLE_NAME.to_owned(),
        ))
        .unwrap();
    }

    #[test]
    fn resolc_compile_works() {
        let input = include_str!("../../../../../test-data/resolc/input/compile-input.json");
        let input: ResolcInput = serde_json::from_str(input).unwrap();
        let out = resolc_instance().compile(&input).unwrap();
        println!("out: {:?}", out);
        assert!(!out.has_error());
    }
}
