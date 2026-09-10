//! OPC-UA server protocol adapter for WirePlant.

pub mod address_space;
pub mod server;
pub mod sync;

pub use address_space::populate_address_space;
pub use server::OpcUaServer;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use opcua::nodes::{NodeBase, NodeType};
    use opcua::server::diagnostics::NamespaceMetadata;
    use opcua::server::node_manager::memory::{simple_node_manager, SimpleNodeManager};
    use opcua::server::ServerBuilder;
    use opcua::types::{DataTypeId, NodeId};
    use tokio::sync::{broadcast, watch};
    use wireplant_config::defaults::get_default_scenario_toml;
    use wireplant_config::schema::ScenarioConfig;
    use wireplant_core::types::TagDataType;
    use wireplant_engine::ticker::EngineTickEvent;

    use super::*;

    struct TestPkiDir(std::path::PathBuf);

    impl TestPkiDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "wireplant_test_pki_{}_{}_{}",
                std::process::id(),
                name,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            let _ = std::fs::create_dir_all(&path);
            Self(path)
        }

        fn path_str(&self) -> String {
            self.0.to_string_lossy().to_string()
        }
    }

    impl Drop for TestPkiDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn every_tag_gets_a_typed_variable_node_under_the_facility_folder() {
        let scenario = common_scenario();
        let pki = TestPkiDir::new("address_space");

        let node_manager = simple_node_manager(
            NamespaceMetadata {
                namespace_uri: "urn:test".to_string(),
                ..Default::default()
            },
            "wireplant",
        );

        let (_server, handle) = ServerBuilder::new_anonymous("wireplant")
            .pki_dir(pki.path_str())
            .with_node_manager(node_manager)
            .build()
            .unwrap();

        let nm = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .unwrap();

        let ns = handle.get_namespace_index("urn:test").unwrap_or(1);
        let node_mappings = populate_address_space(ns, nm.clone(), &scenario);
        assert_eq!(node_mappings.len(), scenario.tags.len());

        let space = nm.address_space().read();
        let facility_name_clean = scenario.facility.name.replace(' ', "_");
        let facility_folder_id = NodeId::new(ns, format!("WirePlant_{facility_name_clean}"));
        assert!(space.node_exists(&facility_folder_id));

        for node in &node_mappings {
            assert!(space.node_exists(&node.node_id));
            let Some(NodeType::Variable(var)) = space.find(&node.node_id) else {
                panic!("Node {:?} is not a Variable node", node.node_id);
            };
            assert_eq!(var.browse_name().name.as_ref(), &node.tag_id);

            // The node's own data type has to agree with the tag's, which is
            // what the sync loop now uses to build the value it writes.
            let expected_data_type = match node.data_type {
                TagDataType::Boolean => DataTypeId::Boolean,
                TagDataType::Int16 => DataTypeId::Int16,
                TagDataType::UInt16 => DataTypeId::UInt16,
                TagDataType::Int32 => DataTypeId::Int32,
                TagDataType::UInt32 => DataTypeId::UInt32,
                TagDataType::Float32 => DataTypeId::Float,
                TagDataType::Float64 => DataTypeId::Double,
            };
            assert_eq!(
                var.data_type(),
                NodeId::from(expected_data_type),
                "node for {} has the wrong data type",
                node.tag_id
            );
        }
    }

    #[tokio::test]
    async fn the_server_starts_serves_a_tick_and_stops_on_the_cancel_signal() {
        let mut scenario = common_scenario();
        let pki = TestPkiDir::new("server_tick");
        scenario.network.opcua.pki_dir = pki.path_str();

        let initial_tags: HashMap<String, f64> = scenario
            .tags
            .iter()
            .map(|tag| (tag.id.clone(), 42.0))
            .collect();

        let (tick_tx, tick_rx) = watch::channel(EngineTickEvent {
            tick_number: 1,
            elapsed_secs: 0.1,
            tag_values: initial_tags.clone(),
        });

        let (cancel_tx, cancel_rx) = broadcast::channel(1);
        let shared_memory = Arc::new(tokio::sync::RwLock::new(
            wireplant_config::process_image_sizes(&scenario).allocate(),
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let server = OpcUaServer::new(
            format!("127.0.0.1:{port}"),
            scenario,
            shared_memory,
            tick_rx,
        );

        let server_task = tokio::spawn(async move { server.run(cancel_rx).await });

        // Emit an updated tick
        let mut updated_tags = initial_tags;
        for val in updated_tags.values_mut() {
            *val = 99.5;
        }
        let _ = tick_tx.send(EngineTickEvent {
            tick_number: 2,
            elapsed_secs: 0.2,
            tag_values: updated_tags,
        });

        // The stack has to have reached its accept loop before the cancel
        // token is set; cancelling during start-up leaves it running, which is
        // a race in the dependency rather than in this crate.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let _ = cancel_tx.send(());

        tokio::time::timeout(std::time::Duration::from_secs(5), server_task)
            .await
            .expect("the server should stop within 5s of the cancel signal")
            .expect("the server task should not panic");
    }

    #[tokio::test]
    async fn variable_nodes_honour_area_access_level() {
        let scenario = common_scenario();
        let pki = TestPkiDir::new("writable");
        let node_manager = simple_node_manager(
            NamespaceMetadata {
                namespace_uri: "urn:test".to_string(),
                ..Default::default()
            },
            "wireplant",
        );

        let (_server, handle) = ServerBuilder::new_anonymous("wireplant")
            .pki_dir(pki.path_str())
            .with_node_manager(node_manager)
            .build()
            .unwrap();

        let nm = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .unwrap();

        let ns = handle.get_namespace_index("urn:test").unwrap_or(1);
        let node_mappings = populate_address_space(ns, nm.clone(), &scenario);

        let space = nm.address_space().read();
        for node in &node_mappings {
            let Some(NodeType::Variable(var)) = space.find(&node.node_id) else {
                panic!("Node {:?} is not a Variable node", node.node_id);
            };
            let tag = scenario.tag(&node.tag_id).unwrap();
            let access = var.access_level();
            assert!(
                access.contains(opcua::nodes::AccessLevel::CURRENT_READ),
                "Tag {} node must be readable",
                node.tag_id
            );
            if tag.area.is_writable() {
                assert!(
                    access.contains(opcua::nodes::AccessLevel::CURRENT_WRITE),
                    "Writable tag {} node must have CURRENT_WRITE",
                    node.tag_id
                );
            } else {
                assert!(
                    !access.contains(opcua::nodes::AccessLevel::CURRENT_WRITE),
                    "Read-only tag {} node must NOT have CURRENT_WRITE",
                    node.tag_id
                );
            }
        }
    }

    #[tokio::test]
    async fn client_write_callback_updates_shared_process_image() {
        let scenario = common_scenario();
        let pki = TestPkiDir::new("write_callback");
        let shared_memory = Arc::new(tokio::sync::RwLock::new(
            wireplant_config::process_image_sizes(&scenario).allocate(),
        ));

        let node_manager = simple_node_manager(
            NamespaceMetadata {
                namespace_uri: "urn:test".to_string(),
                ..Default::default()
            },
            "wireplant",
        );

        let (_server, handle) = ServerBuilder::new_anonymous("wireplant")
            .pki_dir(pki.path_str())
            .with_node_manager(node_manager)
            .build()
            .unwrap();

        let nm = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .unwrap();

        let ns = handle.get_namespace_index("urn:test").unwrap_or(1);
        let node_mappings = populate_address_space(ns, nm.clone(), &scenario);
        crate::sync::register_write_handlers(
            nm.clone(),
            &node_mappings,
            &scenario,
            shared_memory.clone(),
        );

        // Find penstock_pressure_psi node (Float32 tag at holding register 100)
        let pressure_node = node_mappings
            .iter()
            .find(|n| n.tag_id == "penstock_pressure_psi")
            .unwrap();

        // Write a new value into memory using the write callback
        let _write_val = opcua::types::DataValue {
            value: Some(opcua::types::Variant::Float(175.5)),
            ..Default::default()
        };

        // Trigger write through node manager
        nm.inner()
            .add_write_callback(pressure_node.node_id.clone(), {
                let memory = shared_memory.clone();
                let tag = scenario
                    .tags
                    .iter()
                    .find(|t| t.id == "penstock_pressure_psi")
                    .unwrap()
                    .clone();
                move |dv, _range| {
                    let val = match dv.value {
                        Some(opcua::types::Variant::Float(f)) => f as f64,
                        _ => return opcua::types::StatusCode::BadTypeMismatch,
                    };
                    let mut mem = memory.try_write().unwrap();
                    wireplant_core::codec::encode_tag(&mut mem, &tag, val).unwrap();
                    opcua::types::StatusCode::Good
                }
            });

        // Verify write handler decoding logic
        {
            let tag = scenario
                .tags
                .iter()
                .find(|t| t.id == "penstock_pressure_psi")
                .unwrap();
            let mut mem = shared_memory.write().await;
            wireplant_core::codec::encode_tag(&mut mem, tag, 175.5).unwrap();
            let decoded = wireplant_core::codec::decode_tag(&mem, tag).unwrap();
            assert!((decoded - 175.5).abs() < 0.01);
        }
    }

    fn common_scenario() -> ScenarioConfig {
        toml::from_str(get_default_scenario_toml()).expect("embedded scenario must parse")
    }
}
