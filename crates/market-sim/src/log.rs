//! Binary event-log file format.
//!
//! Layout: `QSIM` magic, a bincode header (version, seed, scenario hash),
//! a record count, the bincode records, and a BLAKE3 trailer over everything
//! before it. Readers verify magic, version and trailer — a truncated or
//! bit-flipped log is detected, not silently half-read.

use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sim_core::EventRecord;

const MAGIC: &[u8; 4] = b"QSIM";
const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct LogHeader {
    pub version: u32,
    pub seed: u64,
    /// BLAKE3 of the canonicalized scenario configuration (hex).
    pub scenario_hash: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("encode: {0}")]
    Encode(#[from] bincode::error::EncodeError),
    #[error("decode: {0}")]
    Decode(#[from] bincode::error::DecodeError),
    #[error("not a QSIM log (bad magic)")]
    BadMagic,
    #[error("unsupported log version {0}")]
    BadVersion(u32),
    #[error("integrity check failed (trailer hash mismatch)")]
    BadTrailer,
}

/// Write header + records + integrity trailer to `path` (atomic-ish: written
/// to a sibling `.tmp` then renamed).
pub fn write_log(path: &Path, header: &LogHeader, records: &[EventRecord]) -> Result<(), LogError> {
    let config = bincode::config::standard();
    let mut body: Vec<u8> = Vec::with_capacity(records.len() * 64 + 128);
    body.extend_from_slice(MAGIC);
    body.extend_from_slice(&bincode::encode_to_vec(header, config)?);
    body.extend_from_slice(&bincode::encode_to_vec(records.len() as u64, config)?);
    for record in records {
        body.extend_from_slice(&bincode::encode_to_vec(record, config)?);
    }
    let trailer = blake3::hash(&body);

    let tmp = path.with_extension("qsim.tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&body)?;
        file.write_all(trailer.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read and fully verify a log written by [`write_log`].
pub fn read_log(path: &Path) -> Result<(LogHeader, Vec<EventRecord>), LogError> {
    let config = bincode::config::standard();
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.len() < 4 + 32 {
        return Err(LogError::BadMagic);
    }
    let (body, trailer) = bytes.split_at(bytes.len() - 32);
    if blake3::hash(body).as_bytes() != trailer {
        return Err(LogError::BadTrailer);
    }
    if &body[..4] != MAGIC {
        return Err(LogError::BadMagic);
    }
    let mut cursor = 4usize;
    let (header, used): (LogHeader, usize) = bincode::decode_from_slice(&body[cursor..], config)?;
    cursor += used;
    if header.version != VERSION {
        return Err(LogError::BadVersion(header.version));
    }
    let (count, used): (u64, usize) = bincode::decode_from_slice(&body[cursor..], config)?;
    cursor += used;
    let mut records = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    for _ in 0..count {
        let (record, used): (EventRecord, usize) =
            bincode::decode_from_slice(&body[cursor..], config)?;
        cursor += used;
        records.push(record);
    }
    Ok((header, records))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::{EngineEvent, EventSeq, OrderId, RejectReason, SimTime, SymbolId};

    fn sample() -> Vec<EventRecord> {
        (0..5)
            .map(|i| EventRecord {
                seq: EventSeq::new(i),
                ts: SimTime::from_micros(i),
                symbol: SymbolId::new(0),
                event: EngineEvent::OrderRejected {
                    id: OrderId::new(i),
                    reason: RejectReason::ZeroQty,
                },
            })
            .collect()
    }

    #[test]
    fn round_trips_and_detects_corruption() {
        let dir = std::env::temp_dir().join("quant-sim-log-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("run.qsim");
        let header = LogHeader {
            version: 1,
            seed: 42,
            scenario_hash: "abc".into(),
        };
        let records = sample();
        write_log(&path, &header, &records).unwrap();
        let (h2, r2) = read_log(&path).unwrap();
        assert_eq!(h2, header);
        assert_eq!(r2, records);
        // Corrupt one byte → trailer mismatch.
        let mut bytes = std::fs::read(&path).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();
        assert!(matches!(read_log(&path), Err(LogError::BadTrailer)));
    }
}
