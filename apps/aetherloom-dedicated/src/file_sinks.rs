use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};

use aetherloom_protocol::MatchResult;

use crate::{
    MatchBuild, PersistenceKind, ReplayCheckpointSink, ReplayChunk, SettlementSink, SinkError,
};

const REPLAY_HEADER_MAGIC: [u8; 8] = *b"AELRPL02";
const REPLAY_HEADER_BYTES: u64 = 48;
const REPLAY_RECORD_MAGIC: [u8; 4] = *b"RPL1";
const REPLAY_RECORD_HEADER_BYTES: usize = 36;
const SETTLEMENT_MAGIC: [u8; 8] = *b"AELSET01";
const MAX_SPOOL_RECORD_BYTES: usize = 64 * 1024 * 1024;

/// Durable append-only local replay handoff.
///
/// Each accepted chunk is framed and `sync_data`-ed before `try_store`
/// succeeds. A torn final record is removed when the spool is reopened; a
/// checksum mismatch in a complete record fails closed. The downstream
/// uploader can deduplicate records by `(kind, first_tick, last_tick, hash)`.
pub struct FileReplaySpool {
    path: PathBuf,
    build: MatchBuild,
    work: Option<SyncSender<ReplayChunk>>,
    acknowledgements: Receiver<ReplayAcknowledgement>,
    in_flight: Option<ReplayRecordKey>,
    worker: Option<JoinHandle<()>>,
}

impl FileReplaySpool {
    pub fn open(root: &Path, build: MatchBuild) -> Result<Self, FileSpoolError> {
        let directory = root.join("replay");
        fs::create_dir_all(&directory).map_err(FileSpoolError::Io)?;
        let path = directory.join(format!(
            "{}-epoch-{}.wal",
            hex_id(build.match_id()),
            build.match_epoch()
        ));
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)
            .map_err(FileSpoolError::Io)?;
        if file.metadata().map_err(FileSpoolError::Io)?.len() == 0 {
            let mut header = Vec::with_capacity(REPLAY_HEADER_BYTES as usize);
            header.extend_from_slice(&REPLAY_HEADER_MAGIC);
            header.extend_from_slice(&build.match_id());
            header.extend_from_slice(&build.content_build_hash());
            header.extend_from_slice(&build.match_epoch().to_le_bytes());
            file.write_all(&header).map_err(FileSpoolError::Io)?;
            file.sync_all().map_err(FileSpoolError::Io)?;
            sync_directory(&directory).map_err(FileSpoolError::Io)?;
        } else {
            validate_and_repair_replay(&mut file, build)?;
        }
        let (work_tx, work_rx) = sync_channel::<ReplayChunk>(1);
        let (ack_tx, ack_rx) = sync_channel::<ReplayAcknowledgement>(1);
        let worker = thread::Builder::new()
            .name("aetherloom-replay-spool".to_owned())
            .spawn(move || replay_writer(file, work_rx, ack_tx))
            .map_err(FileSpoolError::Io)?;
        Ok(Self {
            path,
            build,
            work: Some(work_tx),
            acknowledgements: ack_rx,
            in_flight: None,
            worker: Some(worker),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ReplayCheckpointSink for FileReplaySpool {
    fn try_store(&mut self, chunk: &ReplayChunk) -> Result<(), SinkError> {
        if chunk.match_id != self.build.match_id()
            || chunk.content_build_hash != self.build.content_build_hash()
            || chunk.bytes.len() > MAX_SPOOL_RECORD_BYTES
        {
            return Err(SinkError::Rejected);
        }
        let key = ReplayRecordKey::from_chunk(chunk);
        if let Some(in_flight) = self.in_flight {
            if in_flight != key {
                return Err(SinkError::Backpressure);
            }
            return match self.acknowledgements.try_recv() {
                Ok(acknowledgement) if acknowledgement.key == key => {
                    self.in_flight = None;
                    if acknowledgement.stored {
                        Ok(())
                    } else {
                        Err(SinkError::Unavailable)
                    }
                }
                Ok(_) => {
                    self.in_flight = None;
                    Err(SinkError::Unavailable)
                }
                Err(TryRecvError::Empty) => Err(SinkError::Backpressure),
                Err(TryRecvError::Disconnected) => Err(SinkError::Unavailable),
            };
        }
        let Some(work) = self.work.as_ref() else {
            return Err(SinkError::Unavailable);
        };
        match work.try_send(chunk.clone()) {
            Ok(()) => {
                self.in_flight = Some(key);
                Err(SinkError::Backpressure)
            }
            Err(TrySendError::Full(_)) => Err(SinkError::Backpressure),
            Err(TrySendError::Disconnected(_)) => Err(SinkError::Unavailable),
        }
    }
}

impl Drop for FileReplaySpool {
    fn drop(&mut self) {
        self.work.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReplayRecordKey {
    kind: u8,
    first_tick: u64,
    last_tick: u64,
    length: usize,
    checksum: u64,
}

impl ReplayRecordKey {
    fn from_chunk(chunk: &ReplayChunk) -> Self {
        let kind = persistence_kind(chunk.kind);
        Self {
            kind,
            first_tick: chunk.first_tick,
            last_tick: chunk.last_tick,
            length: chunk.bytes.len(),
            checksum: replay_checksum(kind, chunk.first_tick, chunk.last_tick, &chunk.bytes),
        }
    }
}

struct ReplayAcknowledgement {
    key: ReplayRecordKey,
    stored: bool,
}

fn replay_writer(
    mut file: File,
    work: Receiver<ReplayChunk>,
    acknowledgements: SyncSender<ReplayAcknowledgement>,
) {
    let mut available = true;
    while let Ok(chunk) = work.recv() {
        let key = ReplayRecordKey::from_chunk(&chunk);
        let stored = available && write_replay_record(&mut file, &chunk, key).is_ok();
        available = stored;
        if acknowledgements
            .send(ReplayAcknowledgement { key, stored })
            .is_err()
        {
            break;
        }
    }
}

fn write_replay_record(
    file: &mut File,
    chunk: &ReplayChunk,
    key: ReplayRecordKey,
) -> io::Result<()> {
    let length = u32::try_from(key.length)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "replay record is too large"))?;
    let mut record = Vec::with_capacity(REPLAY_RECORD_HEADER_BYTES + chunk.bytes.len());
    record.extend_from_slice(&REPLAY_RECORD_MAGIC);
    record.push(key.kind);
    record.extend_from_slice(&[0; 3]);
    record.extend_from_slice(&key.first_tick.to_le_bytes());
    record.extend_from_slice(&key.last_tick.to_le_bytes());
    record.extend_from_slice(&length.to_le_bytes());
    record.extend_from_slice(&key.checksum.to_le_bytes());
    record.extend_from_slice(&chunk.bytes);
    file.write_all(&record)?;
    file.sync_data()
}

/// Atomic, idempotent local settlement handoff.
///
/// A result is encoded to a temporary file, synced, and linked into the ready
/// directory without replacing an existing idempotency key. Repeating an
/// identical result succeeds; conflicting content is rejected.
pub struct FileSettlementSpool {
    directory: PathBuf,
    temporary_sequence: u64,
}

impl FileSettlementSpool {
    pub fn open(root: &Path) -> Result<Self, FileSpoolError> {
        let directory = root.join("settlements");
        fs::create_dir_all(&directory).map_err(FileSpoolError::Io)?;
        sync_directory(root).map_err(FileSpoolError::Io)?;
        Ok(Self {
            directory,
            temporary_sequence: 0,
        })
    }

    pub fn ready_path(&self, result_id: [u8; 16]) -> PathBuf {
        self.directory.join(format!("{}.ready", hex_id(result_id)))
    }
}

impl SettlementSink for FileSettlementSpool {
    fn try_settle(&mut self, result: &MatchResult) -> Result<(), SinkError> {
        if result.validate().is_err() {
            return Err(SinkError::Rejected);
        }
        let encoded = encode_match_result(result)?;
        let ready = self.ready_path(result.result_id);
        if ready.exists() {
            return compare_existing(&ready, &encoded);
        }
        self.temporary_sequence = self.temporary_sequence.wrapping_add(1);
        let temporary = self.directory.join(format!(
            ".{}.{}.{}.tmp",
            hex_id(result.result_id),
            std::process::id(),
            self.temporary_sequence
        ));
        let write_result = write_synced_new_file(&temporary, &encoded);
        if write_result.is_err() {
            return Err(SinkError::Unavailable);
        }
        match fs::hard_link(&temporary, &ready) {
            Ok(()) => {
                let _ = fs::remove_file(&temporary);
                sync_directory(&self.directory).map_err(|_| SinkError::Unavailable)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&temporary);
                compare_existing(&ready, &encoded)
            }
            Err(_) => {
                let _ = fs::remove_file(&temporary);
                Err(SinkError::Unavailable)
            }
        }
    }
}

#[derive(Debug)]
pub enum FileSpoolError {
    Io(io::Error),
    WrongReplayIdentity,
    CorruptReplay(&'static str),
}

impl fmt::Display for FileSpoolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "spool I/O failed: {error}"),
            Self::WrongReplayIdentity => {
                formatter.write_str("replay spool belongs to another match, build, or epoch")
            }
            Self::CorruptReplay(message) => write!(formatter, "replay spool is corrupt: {message}"),
        }
    }
}

impl Error for FileSpoolError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::WrongReplayIdentity | Self::CorruptReplay(_) => None,
        }
    }
}

fn validate_and_repair_replay(file: &mut File, build: MatchBuild) -> Result<(), FileSpoolError> {
    file.seek(SeekFrom::Start(0)).map_err(FileSpoolError::Io)?;
    let mut header = [0_u8; REPLAY_HEADER_BYTES as usize];
    file.read_exact(&mut header).map_err(FileSpoolError::Io)?;
    if header[..8] != REPLAY_HEADER_MAGIC
        || header[8..24] != build.match_id()
        || header[24..40] != build.content_build_hash()
        || u64::from_le_bytes(
            header[40..48]
                .try_into()
                .expect("fixed replay header epoch range"),
        ) != build.match_epoch()
    {
        return Err(FileSpoolError::WrongReplayIdentity);
    }
    let file_length = file.metadata().map_err(FileSpoolError::Io)?.len();
    let mut good_end = REPLAY_HEADER_BYTES;
    loop {
        let remaining = file_length.saturating_sub(good_end);
        if remaining == 0 {
            break;
        }
        if remaining < REPLAY_RECORD_HEADER_BYTES as u64 {
            repair_torn_tail(file, good_end)?;
            break;
        }
        file.seek(SeekFrom::Start(good_end))
            .map_err(FileSpoolError::Io)?;
        let mut record_header = [0_u8; REPLAY_RECORD_HEADER_BYTES];
        file.read_exact(&mut record_header)
            .map_err(FileSpoolError::Io)?;
        if record_header[..4] != REPLAY_RECORD_MAGIC {
            return Err(FileSpoolError::CorruptReplay("bad record magic"));
        }
        let kind = record_header[4];
        if kind > 1 || record_header[5..8] != [0; 3] {
            return Err(FileSpoolError::CorruptReplay("bad record kind"));
        }
        let first_tick = u64::from_le_bytes(
            record_header[8..16]
                .try_into()
                .expect("fixed record header range"),
        );
        let last_tick = u64::from_le_bytes(
            record_header[16..24]
                .try_into()
                .expect("fixed record header range"),
        );
        let payload_length = u32::from_le_bytes(
            record_header[24..28]
                .try_into()
                .expect("fixed record header range"),
        ) as usize;
        if payload_length > MAX_SPOOL_RECORD_BYTES {
            return Err(FileSpoolError::CorruptReplay("record is oversized"));
        }
        let expected_checksum = u64::from_le_bytes(
            record_header[28..36]
                .try_into()
                .expect("fixed record header range"),
        );
        let record_end = good_end
            .saturating_add(REPLAY_RECORD_HEADER_BYTES as u64)
            .saturating_add(payload_length as u64);
        if record_end > file_length {
            repair_torn_tail(file, good_end)?;
            break;
        }
        let mut payload = vec![0_u8; payload_length];
        file.read_exact(&mut payload).map_err(FileSpoolError::Io)?;
        if replay_checksum(kind, first_tick, last_tick, &payload) != expected_checksum {
            return Err(FileSpoolError::CorruptReplay("record checksum mismatch"));
        }
        good_end = record_end;
    }
    file.seek(SeekFrom::End(0)).map_err(FileSpoolError::Io)?;
    Ok(())
}

fn repair_torn_tail(file: &mut File, good_end: u64) -> Result<(), FileSpoolError> {
    file.set_len(good_end).map_err(FileSpoolError::Io)?;
    file.sync_all().map_err(FileSpoolError::Io)
}

fn persistence_kind(kind: PersistenceKind) -> u8 {
    match kind {
        PersistenceKind::TickRecord => 0,
        PersistenceKind::Checkpoint => 1,
    }
}

fn replay_checksum(kind: u8, first_tick: u64, last_tick: u64, payload: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in [kind]
        .into_iter()
        .chain(first_tick.to_le_bytes())
        .chain(last_tick.to_le_bytes())
        .chain(payload.iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn encode_match_result(result: &MatchResult) -> Result<Vec<u8>, SinkError> {
    let player_count = u16::try_from(result.players.len()).map_err(|_| SinkError::Rejected)?;
    let mut bytes = Vec::with_capacity(64 + result.players.len() * 32);
    bytes.extend_from_slice(&SETTLEMENT_MAGIC);
    bytes.extend_from_slice(&result.result_id);
    bytes.extend_from_slice(&result.match_id);
    bytes.extend_from_slice(&result.completed_tick.to_le_bytes());
    bytes.extend_from_slice(&player_count.to_le_bytes());
    for player in &result.players {
        let loot_count =
            u16::try_from(player.banked_loot.len()).map_err(|_| SinkError::Rejected)?;
        bytes.extend_from_slice(&player.player_id.get().to_le_bytes());
        bytes.extend_from_slice(&player.team_id.get().to_le_bytes());
        bytes.push(player.outcome as u8);
        bytes.extend_from_slice(&player.rating_delta.to_le_bytes());
        bytes.extend_from_slice(&player.score.to_le_bytes());
        bytes.extend_from_slice(&loot_count.to_le_bytes());
        for loot in &player.banked_loot {
            bytes.extend_from_slice(&loot.item_id.to_le_bytes());
            bytes.extend_from_slice(&loot.quantity.to_le_bytes());
        }
    }
    Ok(bytes)
}

fn write_synced_new_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn compare_existing(path: &Path, expected: &[u8]) -> Result<(), SinkError> {
    match fs::read(path) {
        Ok(actual) if actual == expected => Ok(()),
        Ok(_) => Err(SinkError::Rejected),
        Err(_) => Err(SinkError::Unavailable),
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
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

#[cfg(test)]
mod tests {
    use super::*;
    use aetherloom_protocol::{MatchOutcome, PlayerId, PlayerMatchResult, TeamId};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    fn build() -> MatchBuild {
        MatchBuild::new([1; 16], [2; 16], 3).expect("build")
    }

    fn chunk(tick: u64) -> ReplayChunk {
        ReplayChunk {
            match_id: build().match_id(),
            content_build_hash: build().content_build_hash(),
            first_tick: tick,
            last_tick: tick,
            kind: PersistenceKind::TickRecord,
            bytes: vec![4, 5, tick as u8],
        }
    }

    fn result(completed_tick: u64) -> MatchResult {
        MatchResult {
            result_id: [9; 16],
            match_id: build().match_id(),
            completed_tick,
            players: vec![PlayerMatchResult {
                player_id: PlayerId::new(0).expect("player"),
                team_id: TeamId::new(0).expect("team"),
                outcome: MatchOutcome::Defeated,
                rating_delta: 0,
                score: 0,
                banked_loot: Vec::new(),
            }],
        }
    }

    #[test]
    fn replay_spool_survives_reopen_and_rejects_another_build() {
        let root = unique_temp_directory("replay");
        let mut spool = FileReplaySpool::open(&root, build()).expect("open replay");
        store_replay(&mut spool, &chunk(7));
        let path = spool.path().to_owned();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("01010101010101010101010101010101-epoch-3.wal")
        );
        drop(spool);
        FileReplaySpool::open(&root, build()).expect("reopen replay");
        let wrong = MatchBuild::new([1; 16], [8; 16], 3).expect("wrong build");
        assert!(matches!(
            FileReplaySpool::open(&root, wrong),
            Err(FileSpoolError::WrongReplayIdentity)
        ));
        assert!(fs::metadata(path).expect("metadata").len() > REPLAY_HEADER_BYTES);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn replay_spool_rejects_an_epoch_mismatch_in_its_header() {
        let root = unique_temp_directory("epoch");
        let spool = FileReplaySpool::open(&root, build()).expect("open replay");
        let path = spool.path().to_owned();
        drop(spool);

        let mut file = OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open header");
        file.seek(SeekFrom::Start(40)).expect("seek epoch");
        file.write_all(&4_u64.to_le_bytes()).expect("replace epoch");
        file.sync_all().expect("sync epoch");
        drop(file);

        assert!(matches!(
            FileReplaySpool::open(&root, build()),
            Err(FileSpoolError::WrongReplayIdentity)
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn replay_reopen_removes_only_a_torn_tail() {
        let root = unique_temp_directory("torn");
        let mut spool = FileReplaySpool::open(&root, build()).expect("open replay");
        store_replay(&mut spool, &chunk(1));
        let path = spool.path().to_owned();
        drop(spool);
        let good_length = fs::metadata(&path).expect("metadata").len();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("append")
            .write_all(b"RPL1")
            .expect("torn tail");
        FileReplaySpool::open(&root, build()).expect("repair replay");
        assert_eq!(fs::metadata(path).expect("metadata").len(), good_length);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn settlement_spool_is_atomic_and_idempotent() {
        let root = unique_temp_directory("settlement");
        fs::create_dir_all(&root).expect("root");
        let mut spool = FileSettlementSpool::open(&root).expect("open settlements");
        spool.try_settle(&result(10)).expect("settle");
        spool.try_settle(&result(10)).expect("idempotent retry");
        assert_eq!(spool.try_settle(&result(11)), Err(SinkError::Rejected));
        assert!(spool.ready_path([9; 16]).is_file());
        fs::remove_dir_all(root).expect("cleanup");
    }

    fn unique_temp_directory(label: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "aetherloom-spool-{label}-{}-{timestamp}-{sequence}",
            std::process::id()
        ))
    }

    fn store_replay(spool: &mut FileReplaySpool, chunk: &ReplayChunk) {
        for _ in 0..5_000 {
            match spool.try_store(chunk) {
                Ok(()) => return,
                Err(SinkError::Backpressure) => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(error) => panic!("replay store failed: {error:?}"),
            }
        }
        panic!("replay store did not acknowledge durable storage");
    }
}
