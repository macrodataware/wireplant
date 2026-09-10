use opcua::nodes::{AccessLevel, VariableBuilder};
use opcua::server::node_manager::memory::{InMemoryNodeManager, SimpleNodeManagerImpl};
use opcua::types::{DataTypeId, NodeId, Variant};
use std::sync::Arc;
use tracing::info;
use wireplant_config::schema::ScenarioConfig;
use wireplant_core::types::TagDataType;

/// One scenario tag and the address-space node that carries it.
#[derive(Debug, Clone, PartialEq)]
pub struct TagNode {
    pub tag_id: String,
    pub node_id: NodeId,
    pub data_type: TagDataType,
}

/// Populates the OPC-UA address space with facility folders and variable nodes
/// corresponding to tags in the provided scenario configuration.
///
/// Returns the tag, its node and its declared type: the sync loop needs the
/// type to put a value of the right shape into the node it was given.
pub fn populate_address_space(
    ns: u16,
    manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
    scenario: &ScenarioConfig,
) -> Vec<TagNode> {
    let address_space = manager.address_space();
    let mut space = address_space.write();
    let mut registered_nodes = Vec::with_capacity(scenario.tags.len());

    // Create root facility folder under Objects
    let facility_name_clean = scenario.facility.name.replace(' ', "_");
    let facility_folder_id = NodeId::new(ns, format!("WirePlant_{facility_name_clean}"));
    let objects_folder_id = NodeId::objects_folder_id();

    let _ = space.add_folder(
        &facility_folder_id,
        &scenario.facility.name,
        &scenario.facility.name,
        &objects_folder_id,
    );

    for tag in &scenario.tags {
        let tag_node_id = NodeId::new(ns, format!("{facility_name_clean}.{}", tag.id));

        let access = if tag.area.is_writable() {
            AccessLevel::CURRENT_READ | AccessLevel::CURRENT_WRITE
        } else {
            AccessLevel::CURRENT_READ
        };

        let (initial_variant, data_type_id) = match tag.data_type {
            TagDataType::Boolean => (Variant::Boolean(false), DataTypeId::Boolean),
            TagDataType::Float32 => (Variant::Float(0.0), DataTypeId::Float),
            TagDataType::Float64 => (Variant::Double(0.0), DataTypeId::Double),
            TagDataType::Int16 => (Variant::Int16(0), DataTypeId::Int16),
            TagDataType::Int32 => (Variant::Int32(0), DataTypeId::Int32),
            TagDataType::UInt16 => (Variant::UInt16(0), DataTypeId::UInt16),
            TagDataType::UInt32 => (Variant::UInt32(0), DataTypeId::UInt32),
        };

        let var = VariableBuilder::new(&tag_node_id, &tag.id, &tag.id)
            .description(tag.description.clone().unwrap_or_default())
            .value(initial_variant)
            .data_type(data_type_id)
            .access_level(access)
            .user_access_level(access)
            .organized_by(&facility_folder_id);

        let _ = var.insert(&mut *space);

        registered_nodes.push(TagNode {
            tag_id: tag.id.clone(),
            node_id: tag_node_id,
            data_type: tag.data_type,
        });
    }

    info!("Populated {} OPC-UA variable nodes", registered_nodes.len());
    registered_nodes
}
