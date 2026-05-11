#[cfg(windows)]
mod imp {
    use std::{
        io,
        path::Path,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use terminal_manager_diagnostics::{DiagnosticRequest, DiagnosticResponse};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };

    use crate::diagnostics::config::DiagnosticConfig;
    use crate::diagnostics::events::DiagnosticEventStore;
    use crate::diagnostics::server::{
        handle_request, invalid_request_response, DiagnosticAppContext,
    };
    use crate::state::SharedState;

    pub async fn run(config: DiagnosticConfig, shared: SharedState) -> io::Result<()> {
        let endpoint = config.pipe_path().display().to_string();
        let events = DiagnosticEventStore::default();
        let mut server = Server::bind(config.pipe_path())?;
        loop {
            let connection = server.accept().await?;
            if let Err(err) = serve_connection(
                connection,
                &config.token,
                shared.clone(),
                endpoint.clone(),
                events.clone(),
            )
            .await
            {
                log::warn!("diagnostic connection failed: {err}");
            }
        }
    }

    struct Server {
        path: PathBuf,
        pending: Option<NamedPipeServer>,
    }

    impl Server {
        fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
            let path = path.as_ref().to_path_buf();
            let server = create_instance(&path, true)?;
            Ok(Self {
                path,
                pending: Some(server),
            })
        }

        async fn accept(&mut self) -> io::Result<NamedPipeServer> {
            let server = self
                .pending
                .take()
                .expect("pending pipe instance must exist between accepts");
            server.connect().await?;
            self.pending = Some(create_instance(&self.path, false)?);
            Ok(server)
        }
    }

    fn create_instance(path: &Path, first: bool) -> io::Result<NamedPipeServer> {
        let mut options = ServerOptions::new();
        options.first_pipe_instance(first);
        options.create(path).map_err(|err| {
            if first && err.kind() == io::ErrorKind::PermissionDenied {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "another process owns this diagnostic pipe",
                )
            } else {
                err
            }
        })
    }

    async fn serve_connection(
        connection: NamedPipeServer,
        expected_token: &str,
        shared: SharedState,
        endpoint: String,
        events: DiagnosticEventStore,
    ) -> io::Result<()> {
        let mut reader = BufReader::new(connection);
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).await?;
        let response = if bytes == 0 {
            invalid_request_response("empty diagnostic request")
        } else {
            match serde_json::from_str::<DiagnosticRequest>(&line) {
                Ok(request) => {
                    handle_request(request, expected_token, &events, || DiagnosticAppContext {
                        shared,
                        diagnostic_endpoint: Some(endpoint),
                    })
                }
                Err(err) => invalid_request_response(&format!("invalid diagnostic request: {err}")),
            }
        };

        let writer = reader.get_mut();
        write_response(writer, &response).await
    }

    async fn write_response(
        writer: &mut NamedPipeServer,
        response: &DiagnosticResponse,
    ) -> io::Result<()> {
        let mut encoded = serde_json::to_vec(response).map_err(io::Error::other)?;
        encoded.push(b'\n');
        writer.write_all(&encoded).await?;
        writer.flush().await
    }

    async fn connect(path: impl AsRef<Path>) -> io::Result<NamedPipeClient> {
        ClientOptions::new().open(path.as_ref())
    }

    #[cfg(test)]
    async fn send_request(
        path: impl AsRef<Path>,
        request: serde_json::Value,
    ) -> io::Result<DiagnosticResponse> {
        let mut client = connect_with_retry(path.as_ref()).await?;
        let mut encoded = serde_json::to_vec(&request).map_err(io::Error::other)?;
        encoded.push(b'\n');
        client.write_all(&encoded).await?;
        client.flush().await?;

        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        serde_json::from_str::<DiagnosticResponse>(&line).map_err(io::Error::other)
    }

    #[cfg(test)]
    async fn connect_with_retry(path: &Path) -> io::Result<NamedPipeClient> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match connect(path).await {
                Ok(client) => return Ok(client),
                Err(err) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    let _ = err;
                }
                Err(err) => return Err(err),
            }
        }
    }

    #[cfg(test)]
    fn unique_pipe_name() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("tm-diagnostics-test-{}-{n}", std::process::id())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use terminal_manager_diagnostics::{
            DiagnosticCommand, DiagnosticEventFamily, DiagnosticResponse,
            DIAGNOSTIC_PROTOCOL_VERSION,
        };

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn named_pipe_hello_round_trip_requires_token() {
            let config = DiagnosticConfig {
                pipe_name: unique_pipe_name(),
                token: "secret-token".to_owned(),
            };
            let path = config.pipe_path();
            let shared = std::sync::Arc::new(std::sync::Mutex::new(crate::state::seed_state()));
            let server_task = tokio::spawn(async move {
                let _ = run(config, shared).await;
            });

            let hello = DiagnosticRequest {
                token: "secret-token".to_owned(),
                command: DiagnosticCommand::Hello {
                    required_protocol_version: Some(DIAGNOSTIC_PROTOCOL_VERSION.to_owned()),
                },
                ..Default::default()
            };
            let response = send_request(&path, serde_json::to_value(hello).unwrap())
                .await
                .expect("hello response");

            let DiagnosticResponse::Hello {
                protocol_version,
                app,
                capabilities,
                ..
            } = response
            else {
                panic!("expected hello response");
            };
            assert_eq!(protocol_version, DIAGNOSTIC_PROTOCOL_VERSION);
            assert_eq!(app.process_id, Some(std::process::id()));
            assert!(capabilities.commands.contains(&"hello".to_owned()));
            assert!(capabilities.commands.contains(&"snapshot".to_owned()));
            assert!(capabilities.commands.contains(&"drain_events".to_owned()));

            let snapshot = DiagnosticRequest {
                token: "secret-token".to_owned(),
                command: DiagnosticCommand::Snapshot {
                    reason: "pipe-test".to_owned(),
                    options: Default::default(),
                },
                ..Default::default()
            };
            let response = send_request(&path, serde_json::to_value(snapshot).unwrap())
                .await
                .expect("snapshot response");
            let DiagnosticResponse::Snapshot { snapshot } = response else {
                panic!("expected snapshot response");
            };
            assert_eq!(snapshot.reason, "pipe-test");
            assert_eq!(snapshot.app.pid, Some(std::process::id()));

            let invariants = DiagnosticRequest {
                token: "secret-token".to_owned(),
                command: DiagnosticCommand::EvaluateInvariants {
                    scope: terminal_manager_diagnostics::InvariantScope::All,
                },
                ..Default::default()
            };
            let response = send_request(&path, serde_json::to_value(invariants).unwrap())
                .await
                .expect("invariant response");
            let DiagnosticResponse::InvariantResults { results } = response else {
                panic!("expected invariant response");
            };
            assert!(results
                .iter()
                .any(|result| result.id == "app.active_pane.exists"));

            let unauthorized = DiagnosticRequest {
                token: "wrong".to_owned(),
                command: DiagnosticCommand::Hello {
                    required_protocol_version: Some(DIAGNOSTIC_PROTOCOL_VERSION.to_owned()),
                },
                ..Default::default()
            };
            let response = send_request(&path, serde_json::to_value(unauthorized).unwrap())
                .await
                .expect("unauthorized response");
            let DiagnosticResponse::Error { error } = response else {
                panic!("expected unauthorized response");
            };
            assert_eq!(error.code, "unauthorized");
            assert!(error.details.as_object().unwrap().is_empty());

            server_task.abort();
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn named_pipe_mark_step_flush_and_drain_events() {
            let config = DiagnosticConfig {
                pipe_name: unique_pipe_name(),
                token: "secret-token".to_owned(),
            };
            let path = config.pipe_path();
            let shared = std::sync::Arc::new(std::sync::Mutex::new(crate::state::seed_state()));
            let server_task = tokio::spawn(async move {
                let _ = run(config, shared).await;
            });

            let mark_step = DiagnosticRequest {
                token: "secret-token".to_owned(),
                command: DiagnosticCommand::MarkStep {
                    id: "resize-left".to_owned(),
                    label: "Resize left edge".to_owned(),
                },
                ..Default::default()
            };
            assert!(matches!(
                send_request(&path, serde_json::to_value(mark_step).unwrap())
                    .await
                    .expect("mark step response"),
                DiagnosticResponse::Ack { .. }
            ));

            let deterministic = DiagnosticRequest {
                token: "secret-token".to_owned(),
                command: DiagnosticCommand::PrepareDeterministicMode {
                    options: Default::default(),
                },
                ..Default::default()
            };
            assert!(matches!(
                send_request(&path, serde_json::to_value(deterministic).unwrap())
                    .await
                    .expect("deterministic response"),
                DiagnosticResponse::Ack { .. }
            ));

            let flush = DiagnosticRequest {
                token: "secret-token".to_owned(),
                command: DiagnosticCommand::Flush,
                ..Default::default()
            };
            let response = send_request(&path, serde_json::to_value(flush).unwrap())
                .await
                .expect("flush response");
            let DiagnosticResponse::Flushed {
                events_flushed,
                dropped_events,
            } = response
            else {
                panic!("expected flush response");
            };
            assert_eq!(events_flushed, 2);
            assert_eq!(dropped_events, 0);

            let drain = DiagnosticRequest {
                token: "secret-token".to_owned(),
                command: DiagnosticCommand::DrainEvents { limit: None },
                ..Default::default()
            };
            let response = send_request(&path, serde_json::to_value(drain).unwrap())
                .await
                .expect("events response");
            let DiagnosticResponse::Events {
                events,
                dropped_events,
            } = response
            else {
                panic!("expected events response");
            };
            assert_eq!(dropped_events, 0);
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].test_step_id.as_deref(), Some("resize-left"));
            assert_eq!(events[0].payload.family, DiagnosticEventFamily::TestStep);
            assert_eq!(events[1].payload.target, "diagnostics.deterministic_mode");
            assert_eq!(events[1].test_step_id.as_deref(), Some("resize-left"));

            server_task.abort();
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn named_pipe_missing_token_is_rejected_without_state() {
            let config = DiagnosticConfig {
                pipe_name: unique_pipe_name(),
                token: "secret-token".to_owned(),
            };
            let path = config.pipe_path();
            let shared = std::sync::Arc::new(std::sync::Mutex::new(crate::state::seed_state()));
            let server_task = tokio::spawn(async move {
                let _ = run(config, shared).await;
            });

            let response = send_request(
                &path,
                serde_json::json!({
                    "schema_version": terminal_manager_diagnostics::COMMAND_SCHEMA_VERSION,
                    "command": { "type": "hello" }
                }),
            )
            .await
            .expect("invalid request response");

            let DiagnosticResponse::Error { error } = response else {
                panic!("expected invalid request response");
            };
            assert_eq!(error.code, "invalid_request");
            assert!(error.details.as_object().unwrap().is_empty());
            assert!(!error.message.contains("terminal-manager"));

            server_task.abort();
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;

    use crate::diagnostics::config::DiagnosticConfig;

    pub async fn run(
        _config: DiagnosticConfig,
        _shared: crate::state::SharedState,
    ) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "diagnostic named-pipe transport is only supported on Windows",
        ))
    }
}

pub use imp::run;
