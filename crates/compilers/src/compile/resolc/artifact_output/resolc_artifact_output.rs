use std::{
    borrow::{Borrow, Cow},
    collections::BTreeMap,
    path::Path,
};

use alloy_json_abi::{Constructor, Event, Function, JsonAbi};
use alloy_primitives::{hex, Bytes};
use foundry_compilers_artifacts::{
    BytecodeObject, CompactBytecode, CompactContract, CompactContractBytecode,
    CompactContractBytecodeCow, CompactDeployedBytecode, Contract, SourceFile,
};
use serde::{de::value, Deserialize, Serialize};
use serde_json::Error;
use yansi::Paint;

use crate::ArtifactOutput;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub struct ResolcArtifactOutput();

#[derive(Debug, Serialize, Deserialize)]
pub struct ResolcContractArtifact {
    artifact: revive_solidity::SolcStandardJsonOutput,
}

impl Default for ResolcContractArtifact {
    fn default() -> Self {
        Self {
            artifact: revive_solidity::SolcStandardJsonOutput {
                contracts: None,
                sources: None,
                errors: None,
                version: None,
                long_version: None,
                revive_version: None,
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
            abi: Some(standard_abi)
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
        source_file: Option<&SourceFile>,
    ) -> ResolcContractArtifact {
        todo!("Implement this function converting standard json to revive json")
    }
}

fn create_byte_code(
    value: &ResolcContractArtifact,
) -> (JsonAbi, CompactBytecode, CompactDeployedBytecode) {
    let binding = value.artifact.contracts.clone().unwrap();
    let parent_contract = binding.values()
        .last()
        .and_then(|inner_map| inner_map.values().next())
        .unwrap();

    let abi_array: Vec<serde_json::Value> = serde_json::from_value(parent_contract.clone().abi.unwrap()).unwrap();
    let mut standard_abi = JsonAbi {
        constructor: None,
        fallback: None,
        receive: None,
        functions: BTreeMap::new(),
        events: BTreeMap::new(),
        errors: BTreeMap::new(),
    };

    for item in abi_array {
        match item["type"].as_str() {
            Some("constructor") => standard_abi.constructor = serde_json::from_value(item).unwrap(),
            Some("fallback") => standard_abi.fallback = serde_json::from_value(item).unwrap(),
            Some("receive") => standard_abi.receive = serde_json::from_value(item).unwrap(),
            Some("function") => {
                let function: Function = serde_json::from_value(item).unwrap();
                standard_abi.functions
                    .entry(function.name.clone())
                    .or_insert_with(Vec::new)
                    .push(function);
            }
            Some("event") => {
                let event: Event = serde_json::from_value(item).unwrap();
                standard_abi.events
                    .entry(event.name.clone())
                    .or_insert_with(Vec::new)
                    .push(event);
            }
            Some("error") => {
                let error: alloy_json_abi::Error = serde_json::from_value(item).unwrap();
                standard_abi.errors
                    .entry(error.name.clone())
                    .or_insert_with(Vec::new)
                    .push(error);
            }
            _ => continue,
        }
    }

    let binding = parent_contract.evm.clone().unwrap().bytecode.unwrap();
    let raw_bytecode = binding.object.as_str();
    let binding = parent_contract.evm.clone().unwrap().deployed_bytecode.unwrap();
    let raw_deployed_bytecode = binding.object.as_str();

    let bytecode = BytecodeObject::Bytecode(Bytes::from(hex::decode(raw_bytecode).unwrap()));
    let deployed_bytecode = BytecodeObject::Bytecode(Bytes::from(hex::decode(raw_deployed_bytecode).unwrap()));

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