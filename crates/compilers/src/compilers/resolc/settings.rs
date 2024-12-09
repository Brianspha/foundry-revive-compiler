use alloy_primitives::map::HashMap;
use foundry_compilers_artifacts::Remapping;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use crate::{CompilerSettings, CompilerSettingsRestrictions};

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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ResolcRestrictions;

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
        _f: impl FnOnce(&mut foundry_compilers_artifacts::output_selection::OutputSelection) + Copy,
    ) {
        todo!()
    }

    fn can_use_cached(&self, _other: &Self) -> bool {
        todo!()
    }

    fn satisfies_restrictions(&self, _restrictions: &Self::Restrictions) -> bool {
        todo!()
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
