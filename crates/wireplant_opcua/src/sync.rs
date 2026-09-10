use opcua::nodes::NodeType;
use opcua::server::node_manager::memory::{InMemoryNodeManager, SimpleNodeManagerImpl};
use opcua::types::{DataValue, DateTime, StatusCode, Variant};
use std::sync::Arc;
use tokio::sync::watch;
use tracing::debug;
use wireplant_config::schema::ScenarioConfig;
use wireplant_core::codec::{encode_tag, BOOL_THRESHOLD};
use wireplant_core::memory::SharedProcessImage;
use wireplant_core::types::TagDataType;
use wireplant_engine::ticker::EngineTickEvent;

use crate::address_space::TagNode;

/// Registers callbacks to apply OPC-UA client writes back to the shared Process Image.
pub fn register_write_handlers(
    node_manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
    node_mappings: &[TagNode],
    scenario: &ScenarioConfig,
    shared_memory: SharedProcessImage,
) {
    for node in node_mappings {
        let Some(tag) = scenario.tag(&node.tag_id).cloned() else {
            continue;
        };

        // Enforce read-only memory boundary: only register write handlers for writable areas
        if !tag.area.is_writable() {
            continue;
        }

        let memory = shared_memory.clone();
        let tag_id = tag.id.clone();
        let node_id = node.node_id.clone();
        let data_type = node.data_type;
        let address_space = node_manager.address_space().clone();

        node_manager
            .inner()
            .add_write_callback(node.node_id.clone(), move |data_value, _range| {
                let Some(variant) = &data_value.value else {
                    return StatusCode::BadNothingToDo;
                };

                let Some(val_f64) = extract_f64(variant) else {
                    tracing::warn!("OPC-UA write type mismatch for tag '{tag_id}'");
                    return StatusCode::BadTypeMismatch;
                };

                let mut mem = match memory.try_write() {
                    Ok(guard) => guard,
                    Err(_) => {
                        tracing::warn!("Process image lock busy during OPC-UA write to '{tag_id}'");
                        return StatusCode::BadServerHalted;
                    }
                };

                match encode_tag(&mut mem, &tag, val_f64) {
                    Ok(()) => {
                        // Immediately synchronize the OPC-UA address space variable node
                        // so a subsequent read sees the updated value without waiting for the next tick
                        let now = DateTime::now();
                        let fresh_data_value = DataValue {
                            value: Some(variant_for(data_type, val_f64)),
                            status: Some(StatusCode::Good),
                            source_timestamp: Some(now),
                            ..Default::default()
                        };
                        let mut space = address_space.write();
                        if let Some(NodeType::Variable(ref mut var)) = space.find_mut(&node_id) {
                            var.set_data_value(fresh_data_value);
                        }
                        StatusCode::Good
                    }
                    Err(e) => {
                        tracing::error!("Failed to encode OPC-UA write for tag '{tag_id}': {e}");
                        StatusCode::BadOutOfRange
                    }
                }
            });
    }
}

pub fn extract_f64(variant: &Variant) -> Option<f64> {
    match variant {
        Variant::Boolean(b) => Some(if *b { 1.0 } else { 0.0 }),
        Variant::SByte(v) => Some(*v as f64),
        Variant::Byte(v) => Some(*v as f64),
        Variant::Int16(v) => Some(*v as f64),
        Variant::UInt16(v) => Some(*v as f64),
        Variant::Int32(v) => Some(*v as f64),
        Variant::UInt32(v) => Some(*v as f64),
        Variant::Int64(v) => Some(*v as f64),
        Variant::UInt64(v) => Some(*v as f64),
        Variant::Float(v) => Some(*v as f64),
        Variant::Double(v) => Some(*v),
        _ => None,
    }
}

/// Synchronizes telemetry events into OPC-UA variable node values.
pub async fn run_sync_loop(
    manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
    node_mappings: Vec<TagNode>,
    mut tick_rx: watch::Receiver<EngineTickEvent>,
    mut cancel_rx: tokio::sync::broadcast::Receiver<()>,
) {
    let address_space = manager.address_space();

    loop {
        tokio::select! {
            _ = cancel_rx.recv() => {
                debug!("OPC-UA sync loop received cancellation");
                break;
            }
            changed = tick_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                let event = tick_rx.borrow().clone();
                let now = DateTime::now();

                {
                    let mut space = address_space.write();
                    for node in &node_mappings {
                        let Some(value) = event.tag_values.get(&node.tag_id) else {
                            continue;
                        };
                        let data_value = DataValue {
                            // A Double written into an Int16 node is what makes
                            // a typed address space a lie, so the variant comes
                            // from the tag's own declared type.
                            value: Some(variant_for(node.data_type, *value)),
                            status: Some(StatusCode::Good),
                            source_timestamp: Some(now),
                            ..Default::default()
                        };
                        if let Some(NodeType::Variable(ref mut var)) = space.find_mut(&node.node_id) {
                            var.set_data_value(data_value);
                        }
                    }
                }
            }
        }
    }
}

fn variant_for(data_type: TagDataType, value: f64) -> Variant {
    match data_type {
        TagDataType::Boolean => Variant::Boolean(value >= BOOL_THRESHOLD),
        TagDataType::Int16 => Variant::Int16(value as i16),
        TagDataType::UInt16 => Variant::UInt16(value as u16),
        TagDataType::Int32 => Variant::Int32(value as i32),
        TagDataType::UInt32 => Variant::UInt32(value as u32),
        TagDataType::Float32 => Variant::Float(value as f32),
        TagDataType::Float64 => Variant::Double(value),
    }
}
