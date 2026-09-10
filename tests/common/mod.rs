//! Fixtures the integration tests share.
//!
//! The point of this module is that the tests size their Process Image with the
//! same function the binary uses. A test with its own sizing rule cannot catch a
//! sizing bug, which is how `scenario_conformance` used to pass while the
//! binary was laying the tables out differently.

#![allow(dead_code)]

use std::sync::Arc;
use tokio::sync::RwLock;
use wireplant_config::memory::process_image_sizes;
use wireplant_config::schema::ScenarioConfig;
use wireplant_core::memory::SharedProcessImage;

/// Parses an embedded scenario and validates it the way boot does.
pub(crate) fn scenario(toml: &str) -> ScenarioConfig {
    ScenarioConfig::from_toml(toml).expect("embedded scenario must parse and validate")
}

/// A Process Image laid out exactly as the binary lays it out.
pub(crate) fn process_image(config: &ScenarioConfig) -> SharedProcessImage {
    Arc::new(RwLock::new(process_image_sizes(config).allocate()))
}
