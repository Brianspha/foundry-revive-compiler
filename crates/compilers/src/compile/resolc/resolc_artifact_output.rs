use std::{
    borrow::Cow,
    collections::{BTreeMap, HashSet},
    path::Path,
};

use alloy_json_abi::JsonAbi;
use alloy_primitives::{hex, Bytes};
use foundry_compilers_artifacts::{
    BytecodeObject, CompactBytecode, CompactContract, CompactContractBytecode,
    CompactContractBytecodeCow, CompactDeployedBytecode, Contract, SolcLanguage, SourceFile,
};
use path_slash::PathBufExt;
use serde::{Deserialize, Serialize};

use crate::{
    contracts::VersionedContracts, sources::VersionedSourceFiles, ArtifactFile, ArtifactOutput,
    Artifacts, ArtifactsMap, OutputContext, ProjectPathsConfig,
};

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub struct ResolcArtifactOutput();

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResolcContractArtifact {
    artifact: revive_solidity::SolcStandardJsonOutputContract,
}

impl Default for ResolcContractArtifact {
    fn default() -> Self {
        Self {
            artifact: revive_solidity::SolcStandardJsonOutputContract {
                abi: None,
                metadata: None,
                devdoc: None,
                userdoc: None,
                storage_layout: None,
                evm: None,
                ir_optimized: None,
                hash: None,
                factory_dependencies: None,
                missing_libraries: None,
            },
        }
    }
}

impl<'a> From<&'a ResolcContractArtifact> for CompactContractBytecodeCow<'a> {
    fn from(value: &'a ResolcContractArtifact) -> Self {
        let (standard_abi, compact_bytecode, compact_deployed_bytecode) = create_byte_code(value);

        Self {
            abi: Some(Cow::Owned(standard_abi)),
            bytecode: Some(Cow::Owned(compact_bytecode)),
            deployed_bytecode: Some(Cow::Owned(compact_deployed_bytecode)),
        }
    }
}

impl From<ResolcContractArtifact> for CompactContractBytecode {
    fn from(value: ResolcContractArtifact) -> Self {
        let (standard_abi, compact_bytecode, compact_deployed_bytecode) = create_byte_code(&value);
        Self {
            abi: Some(standard_abi),
            bytecode: Some(compact_bytecode),
            deployed_bytecode: Some(compact_deployed_bytecode),
        }
    }
}

impl From<ResolcContractArtifact> for CompactContract {
    fn from(value: ResolcContractArtifact) -> Self {
        let (standard_abi, compact_bytecode, _) = create_byte_code(&value);
        Self {
            bin: Some(compact_bytecode.object.clone()),
            bin_runtime: Some(compact_bytecode.object),
            abi: Some(standard_abi),
        }
    }
}

impl ArtifactOutput for ResolcArtifactOutput {
    type Artifact = ResolcContractArtifact;

    fn contract_to_artifact(
        &self,
        _file: &std::path::Path,
        _name: &str,
        _contract: foundry_compilers_artifacts::Contract,
        _source_file: Option<&foundry_compilers_artifacts::SourceFile>,
    ) -> Self::Artifact {
        todo!("Implement this if needed")
    }

    fn standalone_source_file_to_artifact(
        &self,
        _path: &std::path::Path,
        _file: &crate::sources::VersionedSourceFile,
    ) -> Option<Self::Artifact> {
        None
    }
}

impl ResolcArtifactOutput {
    pub fn resolc_contract_to_artifact(
        &self,
        _file: &Path,
        _name: &str,
        contract: Contract,
        _source_file: Option<&SourceFile>,
    ) -> ResolcContractArtifact {
        ResolcContractArtifact {
            artifact: revive_solidity::SolcStandardJsonOutputContract {
                abi: json_abi_to_revive_abi(contract.abi).unwrap_or_default(),
                metadata: serde_json::from_str(
                    &serde_json::to_string(&contract.metadata).unwrap_or_default(),
                )
                .unwrap_or_default(),
                devdoc: serde_json::from_str(
                    &serde_json::to_string(&contract.devdoc).unwrap_or_default(),
                )
                .unwrap_or_default(),
                userdoc: serde_json::from_str(
                    &serde_json::to_string(&contract.userdoc).unwrap_or_default(),
                )
                .unwrap_or_default(),
                storage_layout: serde_json::from_str(
                    &serde_json::to_string(&contract.storage_layout).unwrap_or_default(),
                )
                .unwrap_or_default(),
                evm: serde_json::from_str(
                    &serde_json::to_string(&contract.evm).unwrap_or_default(),
                )
                .unwrap_or_default(),
                ir_optimized: contract.ir_optimized,
                hash: None,
                factory_dependencies: None,
                missing_libraries: None,
            },
        }
    }
    /// Convert the compiler output into a set of artifacts
    ///
    /// **Note:** This does only convert, but _NOT_ write the artifacts to disk, See
    /// [`Self::on_output()`]
    pub fn resolc_output_to_artifacts(
        &self,
        contracts: &VersionedContracts,
        sources: &VersionedSourceFiles,
        ctx: OutputContext<'_>,
        layout: &ProjectPathsConfig<SolcLanguage>,
    ) -> Artifacts<ResolcContractArtifact> {
        let mut artifacts = ArtifactsMap::new();

        // this tracks all the `SourceFile`s that we successfully mapped to a contract
        let mut non_standalone_sources = HashSet::new();

        // prepopulate taken paths set with cached artifacts
        let mut taken_paths_lowercase = ctx
            .existing_artifacts
            .values()
            .flat_map(|artifacts| artifacts.values())
            .flat_map(|artifacts| artifacts.values())
            .flat_map(|artifacts| artifacts.values())
            .map(|a| a.path.to_slash_lossy().to_lowercase())
            .collect::<HashSet<_>>();

        let mut files = contracts.keys().collect::<Vec<_>>();
        // Iterate starting with top-most files to ensure that they get the shortest paths.
        files.sort_by(|file1, file2| {
            (file1.components().count(), file1).cmp(&(file2.components().count(), file2))
        });
        for file in files {
            for (name, versioned_contracts) in &contracts[file] {
                let unique_versions =
                    versioned_contracts.iter().map(|c| &c.version).collect::<HashSet<_>>();
                let unique_profiles =
                    versioned_contracts.iter().map(|c| &c.profile).collect::<HashSet<_>>();
                for contract in versioned_contracts {
                    non_standalone_sources.insert(file);

                    // track `SourceFile`s that can be mapped to contracts
                    let source_file = sources.find_file_and_version(file, &contract.version);

                    let artifact_path = Self::get_artifact_path(
                        &ctx,
                        &taken_paths_lowercase,
                        file,
                        name,
                        layout.artifacts.as_path(),
                        &contract.version,
                        &contract.profile,
                        unique_versions.len() > 1,
                        unique_profiles.len() > 1,
                    );

                    taken_paths_lowercase.insert(artifact_path.to_slash_lossy().to_lowercase());

                    trace!(
                        "use artifact file {:?} for contract file {} {}",
                        artifact_path,
                        file.display(),
                        contract.version
                    );

                    let artifact = self.resolc_contract_to_artifact(
                        file,
                        name,
                        contract.contract.clone(),
                        source_file,
                    );

                    let artifact = ArtifactFile {
                        artifact,
                        file: artifact_path,
                        version: contract.version.clone(),
                        build_id: contract.build_id.clone(),
                        profile: contract.profile.clone(),
                    };

                    artifacts
                        .entry(file.to_path_buf())
                        .or_default()
                        .entry(name.to_string())
                        .or_default()
                        .push(artifact);
                }
            }
        }

        // extend with standalone source files and convert them to artifacts
        // this is unfortunately necessary, so we can "mock" `Artifacts` for solidity files without
        // any contract definition, which are not included in the `CompilerOutput` but we want to
        // create Artifacts for them regardless
        for (file, sources) in sources.as_ref().iter() {
            let unique_versions = sources.iter().map(|s| &s.version).collect::<HashSet<_>>();
            let unique_profiles = sources.iter().map(|s| &s.profile).collect::<HashSet<_>>();
            for source in sources {
                if !non_standalone_sources.contains(file) {
                    // scan the ast as a safe measure to ensure this file does not include any
                    // source units
                    // there's also no need to create a standalone artifact for source files that
                    // don't contain an ast
                    if source.source_file.ast.is_none()
                        || source.source_file.contains_contract_definition()
                    {
                        continue;
                    }

                    // we use file and file stem
                    if let Some(name) = Path::new(file).file_stem().and_then(|stem| stem.to_str()) {
                        if let Some(artifact) =
                            self.standalone_source_file_to_artifact(file, source)
                        {
                            let artifact_path = Self::get_artifact_path(
                                &ctx,
                                &taken_paths_lowercase,
                                file,
                                name,
                                &layout.artifacts,
                                &source.version,
                                &source.profile,
                                unique_versions.len() > 1,
                                unique_profiles.len() > 1,
                            );

                            taken_paths_lowercase
                                .insert(artifact_path.to_slash_lossy().to_lowercase());

                            artifacts
                                .entry(file.clone())
                                .or_default()
                                .entry(name.to_string())
                                .or_default()
                                .push(ArtifactFile {
                                    artifact,
                                    file: artifact_path,
                                    version: source.version.clone(),
                                    build_id: source.build_id.clone(),
                                    profile: source.profile.clone(),
                                });
                        }
                    }
                }
            }
        }

        Artifacts(artifacts)
    }
}

fn json_abi_to_revive_abi(
    abi: Option<JsonAbi>,
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error>> {
    Ok(abi.map(serde_json::to_value).transpose()?)
}
fn create_byte_code(
    parent_contract: &ResolcContractArtifact,
) -> (JsonAbi, CompactBytecode, CompactDeployedBytecode) {
    let standard_abi = parent_contract
        .artifact
        .abi
        .as_ref()
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_else(|| JsonAbi {
            constructor: None,
            fallback: None,
            receive: None,
            functions: BTreeMap::default(),
            events: BTreeMap::default(),
            errors: BTreeMap::default(),
        });

    let binding = parent_contract.artifact.evm.clone().unwrap().bytecode.unwrap();
    let raw_bytecode = binding.object.as_str();
    let binding = parent_contract.artifact.evm.clone().unwrap().deployed_bytecode.unwrap();
    let raw_deployed_bytecode = binding.object.as_str();

    let bytecode = BytecodeObject::Bytecode(Bytes::from(hex::decode(raw_bytecode).unwrap()));
    let deployed_bytecode =
        BytecodeObject::Bytecode(Bytes::from(hex::decode(raw_deployed_bytecode).unwrap()));

    let compact_bytecode = CompactBytecode {
        object: bytecode,
        source_map: None,
        link_references: BTreeMap::default(),
    };
    let compact_bytecode_deployed = CompactBytecode {
        object: deployed_bytecode,
        source_map: None,
        link_references: BTreeMap::default(),
    };
    let compact_deployed_bytecode = CompactDeployedBytecode {
        bytecode: Some(compact_bytecode_deployed),
        immutable_references: BTreeMap::default(),
    };

    (standard_abi, compact_bytecode, compact_deployed_bytecode)
}
