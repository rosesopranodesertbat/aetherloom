use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use aetherloom_core::WorldSeed;
use aetherloom_protocol::{InputPool, RegionId};
use aetherloom_server::{HostState, TICK_PERIOD};
use quinn::ServerConfig as QuinnServerConfig;
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};

use crate::{HealthReport, MatchAdmissionScope, MatchBuild, ProcessConfig};

pub const DEPLOYMENT_USAGE: &str = "\
Usage: aetherloom-dedicated [options]

Required options (the corresponding AETHERLOOM_* environment variable may be used):
  --match-id HEX                 AETHERLOOM_MATCH_ID
  --content-build-hash HEX       AETHERLOOM_CONTENT_BUILD_HASH
  --match-epoch INTEGER          AETHERLOOM_MATCH_EPOCH
  --region TEXT                  AETHERLOOM_REGION
  --input-pool TEXT              AETHERLOOM_INPUT_POOL
  --result-id HEX                AETHERLOOM_RESULT_ID
  --tls-cert PATH                AETHERLOOM_TLS_CERT
  --tls-key PATH                 AETHERLOOM_TLS_KEY
  --join-public-keys PATH        AETHERLOOM_JOIN_PUBLIC_KEYS
  --ticket-issuer TEXT           AETHERLOOM_TICKET_ISSUER
  --ticket-audience TEXT         AETHERLOOM_TICKET_AUDIENCE
  --spool-dir PATH               AETHERLOOM_SPOOL_DIR

Optional:
  --bind ADDRESS                 AETHERLOOM_BIND (default [::]:4433)
  --health-bind ADDRESS          AETHERLOOM_HEALTH_BIND (default 127.0.0.1:8080)
  --health-file PATH             AETHERLOOM_HEALTH_FILE (default <spool>/health.json)
  --admin-token-file PATH        AETHERLOOM_ADMIN_TOKEN_FILE
  --drain-file PATH              AETHERLOOM_DRAIN_FILE (default <spool>/drain.requested)
  --world-seed INTEGER           AETHERLOOM_WORLD_SEED (default 1)
  --max-players INTEGER          AETHERLOOM_MAX_PLAYERS (default 128)
  --minimum-humans INTEGER       AETHERLOOM_MINIMUM_HUMANS (default 1)
  --lobby-timeout-seconds N      AETHERLOOM_LOBBY_TIMEOUT_SECONDS (default 30)
  --max-match-seconds N          AETHERLOOM_MAX_MATCH_SECONDS (default 7200)
  --drain-grace-seconds N        AETHERLOOM_DRAIN_GRACE_SECONDS (default 30)
  --help
";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DedicatedServerConfig {
    pub process: ProcessConfig,
    pub result_id: [u8; 16],
    pub bind_address: SocketAddr,
    pub health_bind_address: SocketAddr,
    pub tls_certificate_path: PathBuf,
    pub tls_private_key_path: PathBuf,
    pub join_public_keys_path: PathBuf,
    pub ticket_issuer: String,
    pub ticket_audience: String,
    pub spool_directory: PathBuf,
    pub health_file_path: PathBuf,
    pub admin_token_file_path: Option<PathBuf>,
    pub drain_file_path: PathBuf,
    pub minimum_humans: usize,
    pub lobby_timeout: Duration,
    pub maximum_match_duration: Duration,
    pub drain_grace: Duration,
}

impl DedicatedServerConfig {
    pub fn from_env_and_args() -> Result<Self, DeploymentConfigError> {
        Self::parse_with(env::args().skip(1), |name| env::var(name).ok())
    }

    pub fn parse_with<I, S, F>(arguments: I, environment: F) -> Result<Self, DeploymentConfigError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
        F: Fn(&str) -> Option<String>,
    {
        let options = ParsedOptions::parse(arguments)?;
        let match_id = parse_hex_id(
            &options.required("match-id", "AETHERLOOM_MATCH_ID", &environment)?,
            "match id",
        )?;
        let content_build_hash = parse_hex_id(
            &options.required(
                "content-build-hash",
                "AETHERLOOM_CONTENT_BUILD_HASH",
                &environment,
            )?,
            "content build hash",
        )?;
        let match_epoch = parse_number::<u64>(
            &options.required("match-epoch", "AETHERLOOM_MATCH_EPOCH", &environment)?,
            "match epoch",
        )?;
        let result_id = parse_hex_id(
            &options.required("result-id", "AETHERLOOM_RESULT_ID", &environment)?,
            "result id",
        )?;
        if result_id.iter().all(|byte| *byte == 0) {
            return Err(DeploymentConfigError::Invalid(
                "result id must not be zero".to_owned(),
            ));
        }
        let region_value = options.required("region", "AETHERLOOM_REGION", &environment)?;
        let region = RegionId::new(&region_value).map_err(|_| {
            DeploymentConfigError::Invalid("region must be a canonical identifier".to_owned())
        })?;
        let input_pool_value =
            options.required("input-pool", "AETHERLOOM_INPUT_POOL", &environment)?;
        let input_pool = InputPool::from_name(&input_pool_value).map_err(|_| {
            DeploymentConfigError::Invalid(
                "input pool must be browser, mouse-keyboard, controller, or mixed".to_owned(),
            )
        })?;
        let world_seed = parse_number::<u64>(
            &options.value_or("world-seed", "AETHERLOOM_WORLD_SEED", "1", &environment),
            "world seed",
        )?;
        let max_players = parse_number::<u16>(
            &options.value_or("max-players", "AETHERLOOM_MAX_PLAYERS", "128", &environment),
            "max players",
        )?;
        let minimum_humans = parse_number::<usize>(
            &options.value_or(
                "minimum-humans",
                "AETHERLOOM_MINIMUM_HUMANS",
                "1",
                &environment,
            ),
            "minimum humans",
        )?;
        if minimum_humans == 0 {
            return Err(DeploymentConfigError::Invalid(
                "minimum humans must be at least one".to_owned(),
            ));
        }
        if minimum_humans > usize::from(max_players) {
            return Err(DeploymentConfigError::Invalid(
                "minimum humans cannot exceed max players".to_owned(),
            ));
        }

        let build = MatchBuild::new(match_id, content_build_hash, match_epoch)
            .map_err(|error| DeploymentConfigError::Invalid(error.to_string()))?;
        let process = ProcessConfig::new(
            build,
            MatchAdmissionScope::new(region, input_pool),
            WorldSeed::new(world_seed),
            max_players,
        )
        .map_err(|error| DeploymentConfigError::Invalid(error.to_string()))?;
        let spool_directory = required_path(
            options.required("spool-dir", "AETHERLOOM_SPOOL_DIR", &environment)?,
            "spool directory",
        )?;
        let health_file_path = options
            .value("health-file", "AETHERLOOM_HEALTH_FILE", &environment)
            .map(PathBuf::from)
            .unwrap_or_else(|| spool_directory.join("health.json"));
        let drain_file_path = options
            .value("drain-file", "AETHERLOOM_DRAIN_FILE", &environment)
            .map(PathBuf::from)
            .unwrap_or_else(|| spool_directory.join("drain.requested"));
        let admin_token_file_path = options
            .value(
                "admin-token-file",
                "AETHERLOOM_ADMIN_TOKEN_FILE",
                &environment,
            )
            .map(PathBuf::from);

        Ok(Self {
            process,
            result_id,
            bind_address: parse_socket(
                &options.value_or("bind", "AETHERLOOM_BIND", "[::]:4433", &environment),
                "bind address",
            )?,
            health_bind_address: parse_socket(
                &options.value_or(
                    "health-bind",
                    "AETHERLOOM_HEALTH_BIND",
                    "127.0.0.1:8080",
                    &environment,
                ),
                "health bind address",
            )?,
            tls_certificate_path: required_path(
                options.required("tls-cert", "AETHERLOOM_TLS_CERT", &environment)?,
                "TLS certificate path",
            )?,
            tls_private_key_path: required_path(
                options.required("tls-key", "AETHERLOOM_TLS_KEY", &environment)?,
                "TLS private key path",
            )?,
            join_public_keys_path: required_path(
                options.required(
                    "join-public-keys",
                    "AETHERLOOM_JOIN_PUBLIC_KEYS",
                    &environment,
                )?,
                "join public keys path",
            )?,
            ticket_issuer: non_empty(
                options.required("ticket-issuer", "AETHERLOOM_TICKET_ISSUER", &environment)?,
                "ticket issuer",
            )?,
            ticket_audience: non_empty(
                options.required(
                    "ticket-audience",
                    "AETHERLOOM_TICKET_AUDIENCE",
                    &environment,
                )?,
                "ticket audience",
            )?,
            spool_directory,
            health_file_path,
            admin_token_file_path,
            drain_file_path,
            minimum_humans,
            lobby_timeout: parse_seconds(
                &options.value_or(
                    "lobby-timeout-seconds",
                    "AETHERLOOM_LOBBY_TIMEOUT_SECONDS",
                    "30",
                    &environment,
                ),
                "lobby timeout",
                true,
            )?,
            maximum_match_duration: parse_seconds(
                &options.value_or(
                    "max-match-seconds",
                    "AETHERLOOM_MAX_MATCH_SECONDS",
                    "7200",
                    &environment,
                ),
                "maximum match duration",
                false,
            )?,
            drain_grace: parse_seconds(
                &options.value_or(
                    "drain-grace-seconds",
                    "AETHERLOOM_DRAIN_GRACE_SECONDS",
                    "30",
                    &environment,
                ),
                "drain grace",
                false,
            )?,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum DeploymentConfigError {
    HelpRequested,
    Missing(&'static str),
    Invalid(String),
}

impl fmt::Display for DeploymentConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HelpRequested => formatter.write_str(DEPLOYMENT_USAGE),
            Self::Missing(name) => write!(formatter, "missing required option {name}"),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl Error for DeploymentConfigError {}

#[derive(Default)]
struct ParsedOptions {
    values: BTreeMap<String, String>,
}

impl ParsedOptions {
    fn parse<I, S>(arguments: I) -> Result<Self, DeploymentConfigError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut arguments = arguments.into_iter().map(Into::into);
        let mut values = BTreeMap::new();
        while let Some(argument) = arguments.next() {
            if argument == "--help" || argument == "-h" {
                return Err(DeploymentConfigError::HelpRequested);
            }
            let Some(option) = argument.strip_prefix("--") else {
                return Err(DeploymentConfigError::Invalid(format!(
                    "unexpected positional argument {argument}"
                )));
            };
            let (name, value) = match option.split_once('=') {
                Some((name, value)) => (name.to_owned(), value.to_owned()),
                None => {
                    let value = arguments.next().ok_or_else(|| {
                        DeploymentConfigError::Invalid(format!(
                            "option --{option} requires a value"
                        ))
                    })?;
                    (option.to_owned(), value)
                }
            };
            if !KNOWN_OPTIONS.contains(&name.as_str()) {
                return Err(DeploymentConfigError::Invalid(format!(
                    "unknown option --{name}"
                )));
            }
            if value.is_empty() {
                return Err(DeploymentConfigError::Invalid(format!(
                    "option --{name} must not be empty"
                )));
            }
            if values.insert(name.clone(), value).is_some() {
                return Err(DeploymentConfigError::Invalid(format!(
                    "option --{name} was supplied more than once"
                )));
            }
        }
        Ok(Self { values })
    }

    fn value<F>(&self, option: &str, variable: &str, environment: &F) -> Option<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        self.values
            .get(option)
            .cloned()
            .or_else(|| environment(variable))
    }

    fn value_or<F>(&self, option: &str, variable: &str, default: &str, environment: &F) -> String
    where
        F: Fn(&str) -> Option<String>,
    {
        self.value(option, variable, environment)
            .unwrap_or_else(|| default.to_owned())
    }

    fn required<F>(
        &self,
        option: &'static str,
        variable: &str,
        environment: &F,
    ) -> Result<String, DeploymentConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        self.value(option, variable, environment)
            .ok_or(DeploymentConfigError::Missing(option))
    }
}

const KNOWN_OPTIONS: &[&str] = &[
    "match-id",
    "content-build-hash",
    "match-epoch",
    "region",
    "input-pool",
    "result-id",
    "tls-cert",
    "tls-key",
    "join-public-keys",
    "ticket-issuer",
    "ticket-audience",
    "spool-dir",
    "bind",
    "health-bind",
    "health-file",
    "admin-token-file",
    "drain-file",
    "world-seed",
    "max-players",
    "minimum-humans",
    "lobby-timeout-seconds",
    "max-match-seconds",
    "drain-grace-seconds",
];

fn parse_hex_id(value: &str, name: &str) -> Result<[u8; 16], DeploymentConfigError> {
    if value.len() != 32 {
        return Err(DeploymentConfigError::Invalid(format!(
            "{name} must contain exactly 32 hexadecimal characters"
        )));
    }
    let mut output = [0_u8; 16];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0]).ok_or_else(|| {
            DeploymentConfigError::Invalid(format!("{name} contains non-hexadecimal text"))
        })?;
        let low = hex_nibble(pair[1]).ok_or_else(|| {
            DeploymentConfigError::Invalid(format!("{name} contains non-hexadecimal text"))
        })?;
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn parse_number<T>(value: &str, name: &str) -> Result<T, DeploymentConfigError>
where
    T: FromStr,
{
    value
        .parse()
        .map_err(|_| DeploymentConfigError::Invalid(format!("{name} is not a valid integer")))
}

fn parse_seconds(
    value: &str,
    name: &str,
    allow_zero: bool,
) -> Result<Duration, DeploymentConfigError> {
    let seconds = parse_number::<u64>(value, name)?;
    if !allow_zero && seconds == 0 {
        return Err(DeploymentConfigError::Invalid(format!(
            "{name} must be greater than zero"
        )));
    }
    Ok(Duration::from_secs(seconds))
}

fn parse_socket(value: &str, name: &str) -> Result<SocketAddr, DeploymentConfigError> {
    value.parse().map_err(|_| {
        DeploymentConfigError::Invalid(format!("{name} is not a valid socket address"))
    })
}

fn required_path(value: String, name: &str) -> Result<PathBuf, DeploymentConfigError> {
    if value.trim().is_empty() {
        Err(DeploymentConfigError::Invalid(format!(
            "{name} must not be empty"
        )))
    } else {
        Ok(PathBuf::from(value))
    }
}

fn non_empty(value: String, name: &str) -> Result<String, DeploymentConfigError> {
    if value.trim().is_empty() {
        Err(DeploymentConfigError::Invalid(format!(
            "{name} must not be empty"
        )))
    } else {
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LobbyDecision {
    Wait,
    StartMinimumReached,
    StartDeadlineReached,
    AbortNoReservations,
}

pub fn lobby_decision(
    connected_humans: usize,
    admitted_humans: usize,
    minimum_humans: usize,
    elapsed: Duration,
    timeout: Duration,
) -> LobbyDecision {
    if admitted_humans != 0 && connected_humans >= minimum_humans {
        LobbyDecision::StartMinimumReached
    } else if elapsed >= timeout {
        if admitted_humans == 0 {
            LobbyDecision::AbortNoReservations
        } else {
            LobbyDecision::StartDeadlineReached
        }
    } else {
        LobbyDecision::Wait
    }
}

pub fn load_quinn_server_config(
    certificate_path: &Path,
    private_key_path: &Path,
) -> Result<QuinnServerConfig, TlsLoadError> {
    let certificates = CertificateDer::pem_file_iter(certificate_path)
        .map_err(|error| TlsLoadError::Certificate(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| TlsLoadError::Certificate(error.to_string()))?;
    if certificates.is_empty() {
        return Err(TlsLoadError::Certificate(
            "certificate file contains no certificates".to_owned(),
        ));
    }
    let private_key = PrivateKeyDer::from_pem_file(private_key_path)
        .map_err(|error| TlsLoadError::PrivateKey(error.to_string()))?;
    QuinnServerConfig::with_single_cert(certificates, private_key)
        .map_err(|error| TlsLoadError::Tls(error.to_string()))
}

#[derive(Debug, Eq, PartialEq)]
pub enum TlsLoadError {
    Certificate(String),
    PrivateKey(String),
    Tls(String),
}

impl fmt::Display for TlsLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Certificate(message) => write!(formatter, "invalid TLS certificate: {message}"),
            Self::PrivateKey(message) => write!(formatter, "invalid TLS private key: {message}"),
            Self::Tls(message) => write!(formatter, "invalid TLS identity: {message}"),
        }
    }
}

impl Error for TlsLoadError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeploymentPhase {
    Lobby,
    Running,
    Draining,
    Stopped,
    Failed,
}

impl DeploymentPhase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Lobby => "lobby",
            Self::Running => "running",
            Self::Draining => "draining",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeploymentHealth {
    pub phase: DeploymentPhase,
    pub endpoint: SocketAddr,
    pub match_id: [u8; 16],
    pub match_epoch: u64,
    pub report: HealthReport,
    pub failure: Option<String>,
}

impl DeploymentHealth {
    pub fn is_live(&self) -> bool {
        !matches!(
            self.phase,
            DeploymentPhase::Stopped | DeploymentPhase::Failed
        )
    }

    pub fn is_ready_for_players(&self) -> bool {
        self.phase == DeploymentPhase::Lobby
            && self.report.state == HostState::AcceptingPlayers
            && self.report.admission_headroom
    }

    pub fn to_json(&self) -> String {
        let failure = self
            .failure
            .as_deref()
            .map(json_string)
            .unwrap_or_else(|| "null".to_owned());
        format!(
            concat!(
                "{{\"phase\":\"{}\",\"live\":{},\"ready_for_players\":{},",
                "\"endpoint\":\"{}\",\"match_id\":\"{}\",\"match_epoch\":{},",
                "\"content_build_hash\":\"{}\",\"authoritative_tick\":{},",
                "\"connected_players\":{},\"reserved_players\":{},\"max_players\":{},",
                "\"replay_evidence_complete\":{},\"settlement_pending\":{},",
                "\"settlement_finalized\":{},\"failure\":{}}}"
            ),
            self.phase.as_str(),
            self.is_live(),
            self.is_ready_for_players(),
            self.endpoint,
            hex_id(self.match_id),
            self.match_epoch,
            hex_id(self.report.content_build_hash),
            self.report.authoritative_tick,
            self.report.connected_players,
            self.report.reserved_players,
            self.report.max_players,
            self.report.replay_evidence_complete,
            self.report.settlement_pending,
            self.report.settlement_finalized,
            failure,
        )
    }
}

pub struct HealthService {
    address: SocketAddr,
    state: Arc<RwLock<DeploymentHealth>>,
    drain_requested: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl HealthService {
    pub fn bind(
        address: SocketAddr,
        initial: DeploymentHealth,
        admin_token: Option<Vec<u8>>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let state = Arc::new(RwLock::new(initial));
        let drain_requested = Arc::new(AtomicBool::new(false));
        let stop_requested = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_drain = Arc::clone(&drain_requested);
        let worker_stop = Arc::clone(&stop_requested);
        let worker = thread::Builder::new()
            .name("aetherloom-health".to_owned())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            serve_health_request(
                                stream,
                                &worker_state,
                                &worker_drain,
                                admin_token.as_deref(),
                            );
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => {
                            thread::sleep(Duration::from_millis(10));
                        }
                    }
                }
            })?;
        Ok(Self {
            address,
            state,
            drain_requested,
            stop_requested,
            worker: Some(worker),
        })
    }

    pub const fn local_address(&self) -> SocketAddr {
        self.address
    }

    pub fn update(&self, health: DeploymentHealth) {
        if let Ok(mut state) = self.state.write() {
            *state = health;
        }
    }

    pub fn request_drain(&self) {
        self.drain_requested.store(true, Ordering::Release);
    }

    pub fn drain_requested(&self) -> bool {
        self.drain_requested.load(Ordering::Acquire)
    }

    pub fn snapshot(&self) -> Option<DeploymentHealth> {
        self.state.read().ok().map(|state| state.clone())
    }
}

impl Drop for HealthService {
    fn drop(&mut self) {
        self.stop_requested.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn serve_health_request(
    mut stream: TcpStream,
    state: &RwLock<DeploymentHealth>,
    drain_requested: &AtomicBool,
    admin_token: Option<&[u8]>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
    let mut bytes = [0_u8; 4096];
    let Ok(length) = stream.read(&mut bytes) else {
        return;
    };
    if length == 0 {
        return;
    }
    let request = String::from_utf8_lossy(&bytes[..length]);
    let (status, path, body) = health_response(&request, state, drain_requested, admin_token);
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        404 => "Not Found",
        _ => "Service Unavailable",
    };
    let content_type = if path == "/healthz" {
        "application/json"
    } else {
        "text/plain"
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&body);
}

fn health_response(
    request: &str,
    state: &RwLock<DeploymentHealth>,
    drain_requested: &AtomicBool,
    admin_token: Option<&[u8]>,
) -> (u16, String, Vec<u8>) {
    let mut lines = request.lines();
    let Some(request_line) = lines.next() else {
        return (404, String::new(), b"not found\n".to_vec());
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let snapshot = state.read().ok().map(|state| state.clone());
    let (status, body) = match (method, path) {
        ("GET", "/livez") => match snapshot {
            Some(health) if health.is_live() => (200, b"live\n".to_vec()),
            _ => (503, b"not live\n".to_vec()),
        },
        ("GET", "/readyz") => match snapshot {
            Some(health) if health.is_ready_for_players() => (200, b"ready\n".to_vec()),
            _ => (503, b"not ready\n".to_vec()),
        },
        ("GET", "/healthz") => match snapshot {
            Some(health) => (200, health.to_json().into_bytes()),
            None => (503, b"{\"failure\":\"health state unavailable\"}".to_vec()),
        },
        ("POST", "/drain") => {
            let supplied = bearer_token(lines);
            if admin_token.is_some_and(|expected| {
                supplied.is_some_and(|actual| constant_time_equal(expected, actual.as_bytes()))
            }) {
                drain_requested.store(true, Ordering::Release);
                (202, b"draining\n".to_vec())
            } else {
                (404, b"not found\n".to_vec())
            }
        }
        _ => (404, b"not found\n".to_vec()),
    };
    (status, path.to_owned(), body)
}

fn bearer_token<'a>(lines: impl Iterator<Item = &'a str>) -> Option<String> {
    lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .and_then(|(_, value)| value.trim().strip_prefix("Bearer "))
        .map(str::to_owned)
}

fn constant_time_equal(expected: &[u8], actual: &[u8]) -> bool {
    let mut difference = expected.len() ^ actual.len();
    let maximum = expected.len().max(actual.len());
    for index in 0..maximum {
        let left = expected.get(index).copied().unwrap_or(0);
        let right = actual.get(index).copied().unwrap_or(0);
        difference |= usize::from(left ^ right);
    }
    difference == 0
}

pub fn load_admin_token(path: Option<&Path>) -> io::Result<Option<Vec<u8>>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let token = fs::read(path)?;
    let token = trim_ascii_whitespace(&token);
    if token.len() < 32 || token.len() > 512 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "admin token must contain between 32 and 512 non-whitespace bytes",
        ));
    }
    Ok(Some(token.to_vec()))
}

fn trim_ascii_whitespace(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}

pub fn write_health_file(path: &Path, health: &DeploymentHealth) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "health path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(health.to_json().as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    sync_directory(parent)
}

fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
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

fn json_string(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() + 2);
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            character if character.is_control() => {
                use fmt::Write as _;
                let _ = write!(encoded, "\\u{:04x}", character as u32);
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    encoded
}

pub const fn lobby_poll_interval() -> Duration {
    TICK_PERIOD
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    fn required_environment() -> HashMap<String, String> {
        HashMap::from([
            ("AETHERLOOM_MATCH_ID".to_owned(), "01".repeat(16)),
            ("AETHERLOOM_CONTENT_BUILD_HASH".to_owned(), "02".repeat(16)),
            ("AETHERLOOM_MATCH_EPOCH".to_owned(), "42".to_owned()),
            ("AETHERLOOM_REGION".to_owned(), "weur".to_owned()),
            ("AETHERLOOM_INPUT_POOL".to_owned(), "controller".to_owned()),
            ("AETHERLOOM_RESULT_ID".to_owned(), "03".repeat(16)),
            ("AETHERLOOM_TLS_CERT".to_owned(), "/cert.pem".to_owned()),
            ("AETHERLOOM_TLS_KEY".to_owned(), "/key.pem".to_owned()),
            (
                "AETHERLOOM_JOIN_PUBLIC_KEYS".to_owned(),
                "/join-keys.json".to_owned(),
            ),
            (
                "AETHERLOOM_TICKET_ISSUER".to_owned(),
                "aetherloom-staging".to_owned(),
            ),
            (
                "AETHERLOOM_TICKET_AUDIENCE".to_owned(),
                "native-match".to_owned(),
            ),
            ("AETHERLOOM_SPOOL_DIR".to_owned(), "/spool".to_owned()),
        ])
    }

    fn health(phase: DeploymentPhase) -> DeploymentHealth {
        DeploymentHealth {
            phase,
            endpoint: "127.0.0.1:4433".parse().expect("endpoint"),
            match_id: [1; 16],
            match_epoch: 4,
            report: HealthReport {
                state: HostState::AcceptingPlayers,
                content_build_hash: [2; 16],
                authoritative_tick: 7,
                connected_players: 1,
                reserved_players: 1,
                max_players: 16,
                admission_headroom: true,
                egress_packets: 0,
                egress_bytes: 0,
                persistence_chunks: 0,
                persistence_bytes: 0,
                dropped_replay_tick_records: 0,
                dropped_replay_checkpoints: 0,
                replay_evidence_complete: true,
                settlement_pending: false,
                settlement_finalized: false,
                host_receive_failures: 0,
                ingress_quota_drops: 0,
            },
            failure: None,
        }
    }

    #[test]
    fn command_line_overrides_environment_and_validates_lobby() {
        let environment = required_environment();
        let config = DedicatedServerConfig::parse_with(
            [
                "--max-players=32",
                "--minimum-humans",
                "4",
                "--bind",
                "127.0.0.1:5000",
            ],
            |name| environment.get(name).cloned(),
        )
        .expect("configuration");
        assert_eq!(config.process.max_players(), 32);
        assert_eq!(config.process.admission_scope().region().as_str(), "weur");
        assert_eq!(
            config.process.admission_scope().input_pool(),
            InputPool::Controller
        );
        assert_eq!(config.minimum_humans, 4);
        assert_eq!(
            config.bind_address,
            "127.0.0.1:5000".parse().expect("address")
        );
        assert_eq!(config.health_file_path, Path::new("/spool/health.json"));
    }

    #[test]
    fn bad_identifiers_and_unknown_options_fail_closed() {
        let mut environment = required_environment();
        environment.insert("AETHERLOOM_MATCH_ID".to_owned(), "zz".repeat(16));
        assert!(matches!(
            DedicatedServerConfig::parse_with(Vec::<String>::new(), |name| {
                environment.get(name).cloned()
            }),
            Err(DeploymentConfigError::Invalid(_))
        ));
        assert!(matches!(
            DedicatedServerConfig::parse_with(["--typo", "value"], |_| None),
            Err(DeploymentConfigError::Invalid(_))
        ));

        let mut no_humans = required_environment();
        no_humans.insert("AETHERLOOM_MINIMUM_HUMANS".to_owned(), "0".to_owned());
        assert!(matches!(
            DedicatedServerConfig::parse_with(Vec::<String>::new(), |name| {
                no_humans.get(name).cloned()
            }),
            Err(DeploymentConfigError::Invalid(_))
        ));

        let mut wrong_pool = required_environment();
        wrong_pool.insert("AETHERLOOM_INPUT_POOL".to_owned(), "keyboard".to_owned());
        assert!(matches!(
            DedicatedServerConfig::parse_with(Vec::<String>::new(), |name| {
                wrong_pool.get(name).cloned()
            }),
            Err(DeploymentConfigError::Invalid(_))
        ));
    }

    #[test]
    fn lobby_waits_for_minimum_or_deadline() {
        assert_eq!(
            lobby_decision(1, 1, 2, Duration::from_secs(9), Duration::from_secs(10)),
            LobbyDecision::Wait
        );
        assert_eq!(
            lobby_decision(2, 2, 2, Duration::ZERO, Duration::from_secs(10)),
            LobbyDecision::StartMinimumReached
        );
        assert_eq!(
            lobby_decision(0, 1, 2, Duration::from_secs(10), Duration::from_secs(10)),
            LobbyDecision::StartDeadlineReached
        );
        assert_eq!(
            lobby_decision(0, 0, 2, Duration::from_secs(10), Duration::from_secs(10)),
            LobbyDecision::AbortNoReservations
        );
    }

    #[test]
    fn health_json_is_machine_readable_and_escapes_failures() {
        let mut state = health(DeploymentPhase::Failed);
        state.failure = Some("bad \"disk\"\n".to_owned());
        let json = state.to_json();
        assert!(json.contains("\"phase\":\"failed\""));
        assert!(json.contains("\"live\":false"));
        assert!(json.contains("bad \\\"disk\\\"\\n"));
    }

    #[test]
    fn health_service_exposes_readiness_and_authenticated_drain() {
        let state = RwLock::new(health(DeploymentPhase::Lobby));
        let drain = AtomicBool::new(false);
        let token = b"01234567890123456789012345678901";
        let ready = health_response("GET /readyz HTTP/1.1\r\n\r\n", &state, &drain, Some(token));
        assert_eq!(ready.0, 200);
        let hidden = health_response("POST /drain HTTP/1.1\r\n\r\n", &state, &drain, Some(token));
        assert_eq!(hidden.0, 404);
        let accepted = health_response(
            concat!(
                "POST /drain HTTP/1.1\r\n",
                "Authorization: Bearer 01234567890123456789012345678901\r\n\r\n"
            ),
            &state,
            &drain,
            Some(token),
        );
        assert_eq!(accepted.0, 202);
        assert!(drain.load(Ordering::Acquire));
    }

    #[test]
    fn health_file_is_replaced_atomically() {
        let directory = unique_temp_directory("health");
        let path = directory.join("health.json");
        write_health_file(&path, &health(DeploymentPhase::Running)).expect("health file");
        let bytes = fs::read_to_string(&path).expect("read health");
        assert!(bytes.contains("\"phase\":\"running\""));
        fs::remove_dir_all(directory).expect("cleanup");
    }

    fn unique_temp_directory(label: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let sequence = NEXT_TEMP.fetch_add(1, AtomicOrdering::Relaxed);
        env::temp_dir().join(format!(
            "aetherloom-dedicated-{label}-{}-{timestamp}-{sequence}",
            std::process::id()
        ))
    }
}
