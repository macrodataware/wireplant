use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use serde::{Deserialize, Serialize};

/// Record of an active manual override on a tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagOverride {
    pub tag_id: String,
    pub value: f64,
    /// Absolute simulation elapsed time in seconds when this override expires.
    /// `None` represents an indefinite override that persists until explicitly cleared.
    pub expires_at_secs: Option<f64>,
}

/// Registry of forced tag values.
#[derive(Debug, Default, Clone)]
pub struct OverrideRegistry {
    overrides: HashMap<String, TagOverride>,
}

impl OverrideRegistry {
    /// Creates a new empty override registry.
    pub fn new() -> Self {
        Self {
            overrides: HashMap::new(),
        }
    }

    /// Sets or updates a manual override on a tag.
    pub fn set_override(
        &mut self,
        tag_id: impl Into<String>,
        value: f64,
        expires_at_secs: Option<f64>,
    ) {
        let tag_id = tag_id.into();
        self.overrides.insert(
            tag_id.clone(),
            TagOverride {
                tag_id,
                value,
                expires_at_secs,
            },
        );
    }

    /// Removes an active override on a specific tag.
    pub fn remove_override(&mut self, tag_id: &str) -> Option<TagOverride> {
        self.overrides.remove(tag_id)
    }

    /// Clears all active overrides in the registry, returning the number of overrides cleared.
    pub fn clear_all(&mut self) -> usize {
        let count = self.overrides.len();
        self.overrides.clear();
        count
    }

    /// Returns whether the specified tag is currently overridden.
    pub fn is_overridden(&self, tag_id: &str) -> bool {
        self.overrides.contains_key(tag_id)
    }

    /// Retrieves the forced value for a tag if an active override exists.
    pub fn get_value(&self, tag_id: &str) -> Option<f64> {
        self.overrides.get(tag_id).map(|o| o.value)
    }

    /// Returns a full snapshot of all active override values mapped by tag ID.
    pub fn snapshot_values(&self) -> HashMap<String, f64> {
        self.overrides
            .iter()
            .map(|(k, v)| (k.clone(), v.value))
            .collect()
    }

    /// Returns a list of all active tag override records.
    pub fn list_overrides(&self) -> Vec<TagOverride> {
        self.overrides.values().cloned().collect()
    }

    /// Evaluates expiration against current simulation elapsed time, removing expired overrides.
    /// Returns the list of tag IDs that expired during this pass.
    pub fn expire_elapsed(&mut self, current_elapsed_secs: f64) -> Vec<String> {
        let mut expired = Vec::new();
        self.overrides.retain(|tag_id, entry| {
            if let Some(expiry) = entry.expires_at_secs {
                if current_elapsed_secs >= expiry {
                    expired.push(tag_id.clone());
                    return false;
                }
            }
            true
        });
        expired
    }
}

/// Thread-safe shared handle to the manual override registry.
pub type SharedOverrides = Arc<RwLock<OverrideRegistry>>;

/// Helper to create a new shared override registry handle.
pub fn create_shared_overrides() -> SharedOverrides {
    Arc::new(RwLock::new(OverrideRegistry::new()))
}
