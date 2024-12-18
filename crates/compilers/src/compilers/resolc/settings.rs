use alloy_primitives::map::HashMap;
use foundry_compilers_artifacts::Remapping;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use crate::{CompilerSettings, CompilerSettingsRestrictions};

/// This file contains functionality required by revive/resolc
/// Some functions are stubbed but will be implemented as needed
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolcOptimizer {
    pub enabled: bool,
    #[serde(default)]
    pub runs: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct ResolcSettings {
    optimizer: ResolcOptimizer,
    #[serde(rename = "outputSelection")]
    outputselection: HashMap<String, HashMap<String, Vec<String>>>,
}

#[derive(Debug, Clone, Eq, PartialEq, Copy)]
pub enum ResolcRestrictions {
    Default,
}

impl Default for ResolcRestrictions {
    fn default() -> Self {
        Self::Default
    }
}
impl Default for ResolcOptimizer {
    fn default() -> Self {
        Self { enabled: false, runs: 200 }
    }
}

impl CompilerSettingsRestrictions for ResolcRestrictions {
    fn merge(self, _other: Self) -> Option<Self> {
        Some(self)
    }
}

impl CompilerSettings for ResolcSettings {
    type Restrictions = ResolcRestrictions;

    fn update_output_selection(
        &mut self,
        f: impl FnOnce(&mut foundry_compilers_artifacts::output_selection::OutputSelection) + Copy,
    ) {
        let mut output_selection =
            foundry_compilers_artifacts::output_selection::OutputSelection::default();
        f(&mut output_selection);

        let mut selection = HashMap::default();

        for (file, contracts) in output_selection.0 {
            let mut file_outputs = HashMap::default();
            for (contract, outputs) in contracts {
                file_outputs.insert(contract, outputs.into_iter().collect());
            }
            selection.insert(file, file_outputs);
        }

        self.outputselection = selection;
    }

    fn can_use_cached(&self, other: &Self) -> bool {
        self.optimizer == other.optimizer && self.outputselection == other.outputselection
    }

    fn satisfies_restrictions(&self, restrictions: &Self::Restrictions) -> bool {
        match restrictions {
            ResolcRestrictions::Default => true,
        }
    }

    fn with_remappings(self, _remappings: &[Remapping]) -> Self {
        self
    }

    fn with_base_path(self, _base_path: &Path) -> Self {
        self
    }

    fn with_allow_paths(self, _allowed_paths: &BTreeSet<PathBuf>) -> Self {
        self
    }

    fn with_include_paths(self, _include_paths: &BTreeSet<PathBuf>) -> Self {
        self
    }
}

impl ResolcOptimizer {
    pub fn new(enabled: bool, runs: u64) -> Self {
        Self { enabled, runs }
    }
}
impl ResolcSettings {
    pub fn new(
        optimizer: ResolcOptimizer,
        output_selection: HashMap<String, HashMap<String, Vec<String>>>,
    ) -> Self {
        Self { optimizer, outputselection: output_selection }
    }
}
