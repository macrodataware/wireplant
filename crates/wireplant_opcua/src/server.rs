use opcua::server::diagnostics::NamespaceMetadata;
use opcua::server::node_manager::memory::{simple_node_manager, SimpleNodeManager};
use opcua::server::{ServerBuilder, ServerEndpoint, ServerUserToken, ANONYMOUS_USER_TOKEN_ID};
use std::path::Path;
use tokio::sync::watch;
use tracing::{error, info};
use wireplant_config::schema::ScenarioConfig;
use wireplant_core::memory::SharedProcessImage;
use wireplant_engine::ticker::EngineTickEvent;

use crate::address_space::populate_address_space;
use crate::sync::{register_write_handlers, run_sync_loop};

/// Publishes the Process Image as an OPC-UA address space with optional signing and encryption.
#[derive(Debug)]
pub struct OpcUaServer {
    bind_addr: String,
    scenario: ScenarioConfig,
    shared_memory: SharedProcessImage,
    tick_rx: watch::Receiver<EngineTickEvent>,
}

impl OpcUaServer {
    pub fn new(
        bind_addr: impl Into<String>,
        scenario: ScenarioConfig,
        shared_memory: SharedProcessImage,
        tick_rx: watch::Receiver<EngineTickEvent>,
    ) -> Self {
        Self {
            bind_addr: bind_addr.into(),
            scenario,
            shared_memory,
            tick_rx,
        }
    }

    /// Serves until the broadcast fires, then shuts the OPC-UA stack down.
    pub async fn run(
        self,
        mut cancel_rx: tokio::sync::broadcast::Receiver<()>,
    ) -> Result<(), crate::errors::OpcUaError> {
        let (host, port) =
            wireplant_core::net::parse_host_port(&self.bind_addr, 4840).map_err(|e| {
                crate::errors::OpcUaError::InvalidEndpoint {
                    addr: self.bind_addr.clone(),
                    reason: e.to_string(),
                }
            })?;
        let namespace_uri = format!(
            "urn:WirePlant:{}",
            self.scenario.facility.name.replace(' ', "_")
        );
        let discovery_url = format!("opc.tcp://{host}:{port}/");

        let (server, handle) = self.build_server(&host, port, &namespace_uri, &discovery_url)?;

        let node_manager = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .ok_or(crate::errors::OpcUaError::NodeManagerMissing)?;

        let ns = handle.get_namespace_index(&namespace_uri).unwrap_or(1);
        let node_mappings = populate_address_space(ns, node_manager.clone(), &self.scenario);

        // Wire write callbacks back into SharedProcessImage
        register_write_handlers(
            node_manager.clone(),
            &node_mappings,
            &self.scenario,
            self.shared_memory.clone(),
        );

        info!("OPC-UA server listening on opc.tcp://{host}:{port}");

        let sync_cancel = cancel_rx.resubscribe();
        let sync_handle = tokio::spawn(run_sync_loop(
            node_manager,
            node_mappings,
            self.tick_rx,
            sync_cancel,
        ));

        let handle_c = handle.clone();
        let cancel_task = tokio::spawn(async move {
            let _ = cancel_rx.recv().await;
            info!("Cancelling OPC-UA server listener");
            handle_c.cancel();
        });

        if let Err(e) = server.run().await {
            error!("OPC-UA server terminated with error: {e}");
        }

        cancel_task.abort();
        let _ = sync_handle.await;
        Ok(())
    }

    fn build_server(
        &self,
        host: &str,
        port: u16,
        namespace_uri: &str,
        discovery_url: &str,
    ) -> Result<(opcua::server::Server, opcua::server::ServerHandle), crate::errors::OpcUaError>
    {
        let pki_dir = Path::new(&self.scenario.network.opcua.pki_dir);

        let _ = std::fs::create_dir_all(pki_dir.join("own"));
        let _ = std::fs::create_dir_all(pki_dir.join("private"));
        let _ = std::fs::create_dir_all(pki_dir.join("trusted"));
        let _ = std::fs::create_dir_all(pki_dir.join("rejected"));

        let mut user_token_ids: Vec<String> = Vec::new();
        if self.scenario.network.opcua.allow_anonymous {
            user_token_ids.push(ANONYMOUS_USER_TOKEN_ID.to_string());
        }

        let mut builder = ServerBuilder::new()
            .application_name("wireplant")
            .application_uri(namespace_uri.to_string())
            .host(host.to_string())
            .port(port)
            .discovery_urls(vec![discovery_url.to_string()])
            .pki_dir(pki_dir.to_str().unwrap_or("./pki"))
            .trust_client_certs(self.scenario.network.opcua.trust_client_certs)
            .diagnostics_enabled(false);

        for user in &self.scenario.network.opcua.users {
            let token_id = format!("user_{}", user.username);
            builder = builder.add_user_token(
                &token_id,
                ServerUserToken {
                    user: user.username.clone(),
                    pass: Some(user.password.clone()),
                    ..Default::default()
                },
            );
            user_token_ids.push(token_id);
        }

        if self.scenario.network.opcua.allow_anonymous {
            builder = builder.add_endpoint("none", ServerEndpoint::new_none("/", &user_token_ids));
        }

        let has_basic256 = self
            .scenario
            .network
            .opcua
            .security_policies
            .iter()
            .any(|p| p.eq_ignore_ascii_case("Basic256Sha256"));

        let has_aes128 = self
            .scenario
            .network
            .opcua
            .security_policies
            .iter()
            .any(|p| p.eq_ignore_ascii_case("Aes128Sha256RsaOaep"));

        if has_basic256 || has_aes128 {
            builder = builder
                .create_sample_keypair(true)
                .certificate_path("own/cert.der")
                .private_key_path("private/private.pem");

            if has_basic256 {
                builder = builder
                    .add_endpoint(
                        "basic256sha256_sign_encrypt",
                        ServerEndpoint::new_basic256sha256_sign_encrypt("/", &user_token_ids),
                    )
                    .add_endpoint(
                        "basic256sha256_sign",
                        #[allow(deprecated)]
                        ServerEndpoint::new_basic256sha256_sign("/", &user_token_ids),
                    );
            }

            if has_aes128 {
                builder = builder
                    .add_endpoint(
                        "aes128sha256rsaoaep_sign_encrypt",
                        ServerEndpoint::new_aes128_sha256_rsaoaep_sign_encrypt(
                            "/",
                            &user_token_ids,
                        ),
                    )
                    .add_endpoint(
                        "aes128sha256rsaoaep_sign",
                        ServerEndpoint::new_aes128_sha256_rsaoaep_sign("/", &user_token_ids),
                    );
            }
        }

        builder
            .with_node_manager(simple_node_manager(
                NamespaceMetadata {
                    namespace_uri: namespace_uri.to_string(),
                    ..Default::default()
                },
                "wireplant",
            ))
            .build()
            .map_err(crate::errors::OpcUaError::ServerBuild)
    }
}
