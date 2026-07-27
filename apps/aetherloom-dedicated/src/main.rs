use std::error::Error;
use std::fs;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use aetherloom_dedicated::{
    load_admin_token, load_quinn_server_config, lobby_decision, lobby_poll_interval,
    pump_quic_connection_events, write_health_file, DedicatedMatch, DedicatedServerConfig,
    DeploymentConfigError, DeploymentHealth, DeploymentPhase, Ed25519JoinTicketVerifier,
    FileReplaySpool, FileSettlementSpool, HealthService, LobbyDecision, NoDirectTicketVerifier,
    SequentialPeerIdAllocator, SystemUnixTime, TicketConnectionAdmission, DEPLOYMENT_USAGE,
};
use aetherloom_quic::{NativeQuicTransport, QuicTransportConfig};
use aetherloom_server::{DedicatedQuicHost, MatchHost, TickScheduler};

const HEALTH_PUBLISH_INTERVAL: Duration = Duration::from_secs(1);
const TRANSPORT_CLOSE_GRACE: Duration = Duration::from_secs(5);
const HANDOFF_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

fn main() -> ExitCode {
    match DedicatedServerConfig::from_env_and_args() {
        Ok(config) => match run(config) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("aetherloom-dedicated failed: {error}");
                ExitCode::FAILURE
            }
        },
        Err(DeploymentConfigError::HelpRequested) => {
            print!("{DEPLOYMENT_USAGE}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("aetherloom-dedicated configuration error: {error}");
            eprintln!();
            eprintln!("{DEPLOYMENT_USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run(config: DedicatedServerConfig) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(&config.spool_directory)?;
    if config.drain_file_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            format!(
                "drain marker {} already exists; remove it before allocating this match",
                config.drain_file_path.display()
            ),
        )
        .into());
    }

    let public_key_set = fs::read_to_string(&config.join_public_keys_path)?;
    let verifier = Ed25519JoinTicketVerifier::from_public_key_set_json(
        &config.ticket_issuer,
        &config.ticket_audience,
        &public_key_set,
    )?;
    let build = config.process.build();
    let admission = TicketConnectionAdmission::new(
        verifier,
        build,
        config.process.admission_scope(),
        SystemUnixTime,
        SequentialPeerIdAllocator::new(1),
    );
    let tls = load_quinn_server_config(&config.tls_certificate_path, &config.tls_private_key_path)?;
    let transport = NativeQuicTransport::bind(
        tls,
        QuicTransportConfig {
            bind_address: config.bind_address,
            max_connections: usize::from(config.process.max_players()),
            ..QuicTransportConfig::default()
        },
        Arc::new(admission),
    )?;
    let endpoint = transport.local_address();
    let host = DedicatedQuicHost::new(transport)?;
    let replay_spool = FileReplaySpool::open(&config.spool_directory, build)?;
    let settlement_spool = FileSettlementSpool::open(&config.spool_directory)?;
    let mut process = DedicatedMatch::new(
        config.process,
        host,
        NoDirectTicketVerifier,
        replay_spool,
        settlement_spool,
    );

    let initial_health = deployment_health(
        &config,
        endpoint,
        DeploymentPhase::Lobby,
        process.health(),
        None,
    );
    let admin_token = load_admin_token(config.admin_token_file_path.as_deref())?;
    let health_service = HealthService::bind(
        config.health_bind_address,
        initial_health.clone(),
        admin_token,
    )?;
    write_health_file(&config.health_file_path, &initial_health)?;

    println!(
        "Aetherloom match {} listening on QUIC {} with health on {}",
        hex_id(build.match_id()),
        endpoint,
        health_service.local_address()
    );
    println!(
        "Lobby waits for {} connected human(s) or {:?}; no signing or TLS secret was generated.",
        config.minimum_humans, config.lobby_timeout
    );

    let lobby_started = Instant::now();
    let mut last_health_publish = Instant::now();
    let mut fatal_failure = None;
    let mut should_start = false;
    loop {
        if let Err(error) = pump_quic_connection_events(&mut process) {
            fatal_failure = Some(error.to_string());
            break;
        }
        let elapsed = lobby_started.elapsed();
        let decision = lobby_decision(
            process.health().connected_players,
            process.admitted_player_count(),
            config.minimum_humans,
            elapsed,
            config.lobby_timeout,
        );
        match decision {
            LobbyDecision::Wait => {}
            LobbyDecision::StartMinimumReached | LobbyDecision::StartDeadlineReached => {
                println!(
                    "Lobby closed ({decision:?}) with {} connected human(s).",
                    process.health().connected_players
                );
                should_start = true;
                break;
            }
            LobbyDecision::AbortNoReservations => {
                println!(
                    "Lobby deadline expired before any human reservation arrived; cancelling match."
                );
                break;
            }
        }
        if drain_requested(&config, &health_service) {
            println!("Drain requested while in the lobby; match will not start.");
            break;
        }
        if last_health_publish.elapsed() >= HEALTH_PUBLISH_INTERVAL {
            if let Err(error) = publish_health(
                &config,
                endpoint,
                DeploymentPhase::Lobby,
                &process,
                &health_service,
                None,
            ) {
                fatal_failure = Some(error.to_string());
                break;
            }
            last_health_publish = Instant::now();
        }
        thread::sleep(lobby_poll_interval());
    }

    let mut drain_deadline = None;
    if fatal_failure.is_some() || !should_start {
        begin_drain(&mut process);
        drain_deadline = Some(Instant::now());
    }

    if should_start && fatal_failure.is_none() {
        let mut scheduler = TickScheduler::system(4_096);
        let match_started = Instant::now();
        if let Err(error) = publish_health(
            &config,
            endpoint,
            DeploymentPhase::Running,
            &process,
            &health_service,
            None,
        ) {
            fatal_failure = Some(error.to_string());
            begin_drain(&mut process);
            drain_deadline = Some(Instant::now());
        }
        last_health_publish = Instant::now();

        loop {
            if let Err(error) = pump_quic_connection_events(&mut process) {
                fatal_failure = Some(error.to_string());
                begin_drain(&mut process);
                drain_deadline = Some(Instant::now());
            }
            if fatal_failure.is_none() {
                if let Err(error) = process.run_scheduled_tick(&mut scheduler) {
                    fatal_failure = Some(error.to_string());
                    begin_drain(&mut process);
                    drain_deadline = Some(Instant::now());
                }
            }

            if drain_deadline.is_none() {
                let terminal = process.all_admitted_players_terminal();
                let requested = drain_requested(&config, &health_service);
                let expired = match_started.elapsed() >= config.maximum_match_duration;
                if terminal || requested || expired {
                    let reason = if terminal {
                        "all admitted players reached terminal outcomes"
                    } else if requested {
                        "an operator requested drain"
                    } else {
                        "the configured maximum match duration elapsed"
                    };
                    println!("Beginning graceful drain because {reason}.");
                    begin_drain(&mut process);
                    drain_deadline = Some(if terminal {
                        Instant::now()
                    } else {
                        Instant::now() + config.drain_grace
                    });
                }
            }

            let phase = if drain_deadline.is_some() {
                DeploymentPhase::Draining
            } else {
                DeploymentPhase::Running
            };
            if last_health_publish.elapsed() >= HEALTH_PUBLISH_INTERVAL {
                if let Err(error) = publish_health(
                    &config,
                    endpoint,
                    phase,
                    &process,
                    &health_service,
                    fatal_failure.as_deref(),
                ) {
                    fatal_failure.get_or_insert_with(|| error.to_string());
                    begin_drain(&mut process);
                    drain_deadline = Some(Instant::now());
                }
                last_health_publish = Instant::now();
            }

            if drain_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }
        }
    }

    begin_drain(&mut process);
    let abandoned = process.abandon_active_players()?;
    if abandoned != 0 {
        println!("Marked {abandoned} active admitted player(s) abandoned at drain deadline.");
    }
    if process.admitted_player_count() != 0 {
        process.seal_match_result(config.result_id)?;
    }
    let handoff_deadline = Instant::now() + HANDOFF_DRAIN_TIMEOUT;
    while Instant::now() < handoff_deadline {
        process.poll_external_handoffs();
        let health = process.health();
        if health.persistence_chunks == 0 && !health.settlement_pending {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    if process.admitted_player_count() != 0 && !process.health().settlement_finalized {
        fatal_failure.get_or_insert_with(|| {
            "authoritative settlement could not be durably handed off".to_owned()
        });
    }
    if !process.health().replay_evidence_complete {
        fatal_failure.get_or_insert_with(|| {
            "replay evidence is queued or incomplete at shutdown".to_owned()
        });
    }

    let final_phase = if fatal_failure.is_some() {
        DeploymentPhase::Failed
    } else {
        DeploymentPhase::Draining
    };
    publish_health(
        &config,
        endpoint,
        final_phase,
        &process,
        &health_service,
        fatal_failure.as_deref(),
    )?;
    process
        .host_mut()
        .transport_mut()
        .shutdown_gracefully(Instant::now() + TRANSPORT_CLOSE_GRACE);
    process.host_mut().stop();

    let stopped_phase = if fatal_failure.is_some() {
        DeploymentPhase::Failed
    } else {
        DeploymentPhase::Stopped
    };
    publish_health(
        &config,
        endpoint,
        stopped_phase,
        &process,
        &health_service,
        fatal_failure.as_deref(),
    )?;

    if let Some(failure) = fatal_failure {
        Err(io::Error::other(failure).into())
    } else {
        println!("Match stopped with durable replay and settlement handoff.");
        Ok(())
    }
}

fn begin_drain(
    process: &mut DedicatedMatch<
        DedicatedQuicHost<NativeQuicTransport>,
        NoDirectTicketVerifier,
        FileReplaySpool,
        FileSettlementSpool,
    >,
) {
    process.begin_drain();
    process.host_mut().transport_mut().begin_draining();
}

fn drain_requested(config: &DedicatedServerConfig, health: &HealthService) -> bool {
    health.drain_requested() || config.drain_file_path.is_file()
}

fn publish_health(
    config: &DedicatedServerConfig,
    endpoint: std::net::SocketAddr,
    phase: DeploymentPhase,
    process: &DedicatedMatch<
        DedicatedQuicHost<NativeQuicTransport>,
        NoDirectTicketVerifier,
        FileReplaySpool,
        FileSettlementSpool,
    >,
    service: &HealthService,
    failure: Option<&str>,
) -> io::Result<()> {
    let health = deployment_health(
        config,
        endpoint,
        phase,
        process.health(),
        failure.map(str::to_owned),
    );
    service.update(health.clone());
    write_health_file(&config.health_file_path, &health)
}

fn deployment_health(
    config: &DedicatedServerConfig,
    endpoint: std::net::SocketAddr,
    phase: DeploymentPhase,
    report: aetherloom_dedicated::HealthReport,
    failure: Option<String>,
) -> DeploymentHealth {
    DeploymentHealth {
        phase,
        endpoint,
        match_id: config.process.build().match_id(),
        match_epoch: config.process.build().match_epoch(),
        report,
        failure,
    }
}

fn hex_id(value: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(32);
    for byte in value {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
