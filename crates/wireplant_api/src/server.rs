use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tracing::info;
use wireplant_config::schema::ScenarioConfig;
use wireplant_core::memory::{SessionCounter, SharedProcessImage};
use wireplant_engine::alarm::AlarmManager;
use wireplant_engine::historian::Historian;

use std::sync::atomic::AtomicBool;
use tower_http::catch_panic::CatchPanicLayer;

use crate::routes::{
    acknowledge_alarm_handler, clear_fault_handler, get_alarms_handler, get_state_handler,
    health_handler, inject_fault_handler, query_history_handler, reset_alarm_handler,
    shelve_alarm_handler, unshelve_alarm_handler, AppState,
};
use wireplant_core::overrides::{create_shared_overrides, SharedOverrides};

#[derive(Debug)]
pub struct ApiServer {
    scenario: ScenarioConfig,
    shared_memory: SharedProcessImage,
    sessions: SessionCounter,
    alarms: AlarmManager,
    overrides: SharedOverrides,
    historian: Option<Arc<Historian>>,
    engine_liveness: Option<Arc<AtomicBool>>,
}

/// Fluent builder for configuring and assembling an [`ApiServer`].
#[derive(Debug)]
pub struct ApiServerBuilder {
    scenario: ScenarioConfig,
    shared_memory: SharedProcessImage,
    sessions: SessionCounter,
    alarms: Option<AlarmManager>,
    overrides: Option<SharedOverrides>,
    historian: Option<Arc<Historian>>,
    engine_liveness: Option<Arc<AtomicBool>>,
}

impl ApiServerBuilder {
    /// Initializes a builder with the required scenario, process image, and session counter.
    pub fn new(
        scenario: ScenarioConfig,
        shared_memory: SharedProcessImage,
        sessions: SessionCounter,
    ) -> Self {
        Self {
            scenario,
            shared_memory,
            sessions,
            alarms: None,
            overrides: None,
            historian: None,
            engine_liveness: None,
        }
    }

    /// Attaches an existing alarm manager. When omitted, builds one from `scenario.alarms`.
    pub fn with_alarms(mut self, alarms: AlarmManager) -> Self {
        self.alarms = Some(alarms);
        self
    }

    /// Attaches a shared overrides registry. When omitted, creates a new empty registry.
    pub fn with_overrides(mut self, overrides: SharedOverrides) -> Self {
        self.overrides = Some(overrides);
        self
    }

    /// Attaches an optional SQLite historian persistence handle.
    pub fn with_historian(mut self, historian: Option<Arc<Historian>>) -> Self {
        self.historian = historian;
        self
    }

    /// Attaches an engine liveness heartbeat flag for HTTP healthcheck probes.
    pub fn with_engine_liveness(mut self, engine_liveness: Option<Arc<AtomicBool>>) -> Self {
        self.engine_liveness = engine_liveness;
        self
    }

    /// Assembles the [`ApiServer`].
    pub fn build(self) -> ApiServer {
        let alarms = self
            .alarms
            .unwrap_or_else(|| AlarmManager::from_configs(&self.scenario.alarms));
        let overrides = self.overrides.unwrap_or_else(create_shared_overrides);

        ApiServer {
            scenario: self.scenario,
            shared_memory: self.shared_memory,
            sessions: self.sessions,
            alarms,
            overrides,
            historian: self.historian,
            engine_liveness: self.engine_liveness,
        }
    }
}

impl ApiServer {
    /// Returns a fluent builder for configuring and creating an [`ApiServer`].
    pub fn builder(
        scenario: ScenarioConfig,
        shared_memory: SharedProcessImage,
        sessions: SessionCounter,
    ) -> ApiServerBuilder {
        ApiServerBuilder::new(scenario, shared_memory, sessions)
    }

    /// Constructs an [`ApiServer`] with default alarm manager and fresh overrides.
    pub fn new(
        scenario: ScenarioConfig,
        shared_memory: SharedProcessImage,
        sessions: SessionCounter,
    ) -> Self {
        Self::builder(scenario, shared_memory, sessions).build()
    }

    pub fn with_overrides(mut self, overrides: SharedOverrides) -> Self {
        self.overrides = overrides;
        self
    }

    pub fn with_historian(mut self, historian: Option<Arc<Historian>>) -> Self {
        self.historian = historian;
        self
    }

    pub fn with_engine_liveness(mut self, engine_liveness: Option<Arc<AtomicBool>>) -> Self {
        self.engine_liveness = engine_liveness;
        self
    }

    pub fn router(state: Arc<AppState>) -> Router {
        let protected = Router::new()
            .route("/api/v1/state", get(get_state_handler))
            .route("/api/v1/fault", post(inject_fault_handler))
            .route(
                "/api/v1/fault/{tag_id}",
                axum::routing::delete(clear_fault_handler),
            )
            .route("/api/v1/alarms", get(get_alarms_handler))
            .route(
                "/api/v1/alarms/{id}/acknowledge",
                post(acknowledge_alarm_handler),
            )
            .route("/api/v1/alarms/{id}/reset", post(reset_alarm_handler))
            .route("/api/v1/alarms/{id}/shelve", post(shelve_alarm_handler))
            .route("/api/v1/alarms/{id}/unshelve", post(unshelve_alarm_handler))
            .route("/api/v1/history", get(query_history_handler))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::auth::require_auth_middleware,
            ));

        Router::new()
            .route("/health", get(health_handler))
            .route("/metrics", get(crate::metrics::prometheus_metrics_handler))
            .merge(protected)
            .layer(CatchPanicLayer::new())
            .with_state(state)
    }

    /// Serves an already-bound listener until the broadcast fires.
    ///
    /// Binding is the caller's job so the port is known before the task starts.
    pub async fn serve(
        self,
        listener: TcpListener,
        mut cancel_rx: tokio::sync::broadcast::Receiver<()>,
    ) -> Result<(), std::io::Error> {
        let state = Arc::new(AppState {
            scenario: self.scenario,
            shared_memory: self.shared_memory,
            sessions: self.sessions,
            alarms: self.alarms,
            overrides: self.overrides,
            historian: self.historian,
            engine_liveness: self.engine_liveness,
            start_time: Instant::now(),
        });

        let app = Self::router(state);

        info!(
            "Inspection HTTP REST API listening on http://{}",
            listener.local_addr()?
        );

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = cancel_rx.recv().await;
                info!("Inspection API received shutdown signal");
            })
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;
    use wireplant_config::defaults::get_default_scenario_toml;
    use wireplant_core::memory::PlantMemory;

    async fn health_router() -> Router {
        let scenario: ScenarioConfig =
            toml::from_str(get_default_scenario_toml()).expect("embedded scenario must parse");
        let shared_memory = Arc::new(tokio::sync::RwLock::new(PlantMemory::new(10, 10, 10, 100)));

        ApiServer::router(Arc::new(AppState {
            scenario,
            shared_memory,
            sessions: SessionCounter::default(),
            alarms: AlarmManager::default(),
            overrides: create_shared_overrides(),
            historian: None,
            engine_liveness: None,
            start_time: Instant::now(),
        }))
    }

    #[tokio::test]
    async fn health_endpoint_answers_ok() {
        let app = health_router().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    async fn authenticated_router(api_key: Option<&str>) -> Router {
        let mut scenario: ScenarioConfig =
            toml::from_str(get_default_scenario_toml()).expect("embedded scenario must parse");
        scenario.network.http.api_key = api_key.map(|k| k.to_string());
        let shared_memory = Arc::new(tokio::sync::RwLock::new(PlantMemory::new(
            100, 100, 100, 500,
        )));

        ApiServer::router(Arc::new(AppState {
            scenario,
            shared_memory,
            sessions: SessionCounter::default(),
            alarms: AlarmManager::default(),
            overrides: create_shared_overrides(),
            historian: None,
            engine_liveness: None,
            start_time: Instant::now(),
        }))
    }

    #[tokio::test]
    async fn health_remains_public_when_api_key_is_configured() {
        let app = authenticated_router(Some("secret-key")).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn protected_endpoint_refuses_unauthenticated_request() {
        let app = authenticated_router(Some("secret-key")).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn protected_endpoint_accepts_valid_bearer_token() {
        let app = authenticated_router(Some("secret-key")).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/state")
                    .header("Authorization", "Bearer secret-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn protected_endpoint_accepts_valid_x_api_key_header() {
        let app = authenticated_router(Some("secret-key")).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/state")
                    .header("X-API-Key", "secret-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn protected_endpoint_rejects_wrong_token() {
        let app = authenticated_router(Some("secret-key")).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/state")
                    .header("Authorization", "Bearer wrong-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn open_mode_permits_unauthenticated_state_queries() {
        let app = authenticated_router(None).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn health_returns_503_service_unavailable_when_engine_is_not_alive() {
        let scenario: ScenarioConfig =
            toml::from_str(get_default_scenario_toml()).expect("embedded scenario must parse");
        let shared_memory = Arc::new(tokio::sync::RwLock::new(PlantMemory::new(10, 10, 10, 100)));
        let liveness = Arc::new(AtomicBool::new(false)); // dead engine

        let app = ApiServer::router(Arc::new(AppState {
            scenario,
            shared_memory,
            sessions: SessionCounter::default(),
            alarms: AlarmManager::default(),
            overrides: create_shared_overrides(),
            historian: None,
            engine_liveness: Some(liveness),
            start_time: Instant::now(),
        }));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn shelve_alarm_rejects_huge_or_non_finite_duration() {
        let scenario: ScenarioConfig =
            toml::from_str(get_default_scenario_toml()).expect("embedded scenario must parse");
        let shared_memory = Arc::new(tokio::sync::RwLock::new(PlantMemory::new(10, 10, 10, 100)));
        let app = ApiServer::router(Arc::new(AppState {
            scenario,
            shared_memory,
            sessions: SessionCounter::default(),
            alarms: AlarmManager::default(),
            overrides: create_shared_overrides(),
            historian: None,
            engine_liveness: None,
            start_time: Instant::now(),
        }));

        // Send 1e300 duration payload
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/alarms/test_alarm/shelve")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"duration_secs": 1e300}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
