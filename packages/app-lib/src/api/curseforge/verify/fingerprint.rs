//! Murmur2 fingerprinting and sha1 hashing of CurseForge files.

use crate::ErrorKind;
use std::path::Path;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

async fn read_curseforge_verification_chunk(
    file: &mut tokio::fs::File,
    buffer: &mut [u8],
    cancellation: Option<&CancellationToken>,
) -> crate::Result<usize> {
    match cancellation {
        Some(cancellation) => tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(ErrorKind::OtherError(
                "CurseForge fingerprint verification canceled".to_string(),
            ).into()),
            result = file.read(buffer) => Ok(result?),
        },
        None => Ok(file.read(buffer).await?),
    }
}

pub(crate) async fn fingerprint_and_sha1_file(
    path: &Path,
    cancellation: Option<&CancellationToken>,
) -> crate::Result<(u64, String, u32)> {
    #[cfg(test)]
    crate::util::download::verified_file::record_scan();
    let started = std::time::Instant::now();
    let acquire = crate::util::fetch::acquire_native_validation_permit();
    let _permit = if let Some(cancellation) = cancellation {
        tokio::select! { biased; _ = cancellation.cancelled() => return Err(ErrorKind::OtherError("fingerprint verification canceled while waiting for resources".into()).into()), permit = acquire => permit? }
    } else {
        acquire.await?
    };

    use crate::util::download::constants::CURSEFORGE_VERIFY_BUFFER_SIZE as BUFFER_SIZE;
    let mut file = tokio::fs::File::open(path).await?;
    let mut buffer = vec![0_u8; BUFFER_SIZE];
    let mut sha1 = sha1_smol::Sha1::new();
    let mut size = 0_u64;
    let mut normalized_size = 0_u64;
    loop {
        let read = read_curseforge_verification_chunk(
            &mut file,
            &mut buffer,
            cancellation,
        )
        .await?;
        if read == 0 {
            break;
        }
        let bytes = &buffer[..read];
        sha1.update(bytes);
        size += read as u64;
        normalized_size += bytes
            .iter()
            .filter(|byte| !is_curseforge_fingerprint_whitespace(**byte))
            .count() as u64;
    }

    let mut file = tokio::fs::File::open(path).await?;
    let mut fingerprint =
        CurseForgeFingerprintHasher::new(normalized_size as u32);
    loop {
        let read = read_curseforge_verification_chunk(
            &mut file,
            &mut buffer,
            cancellation,
        )
        .await?;
        if read == 0 {
            break;
        }
        fingerprint.update(&buffer[..read]);
    }
    tracing::debug!(path = %path.display(), bytes = size, verification_ms = started.elapsed().as_millis(), "Completed CurseForge fingerprint verification");
    Ok((size, sha1.digest().to_string(), fingerprint.finish()))
}

pub(crate) fn is_curseforge_fingerprint_whitespace(byte: u8) -> bool {
    matches!(byte, 9 | 10 | 13 | 32)
}

pub(crate) struct CurseForgeFingerprintHasher {
    pub(crate) hash: u32,
    pub(crate) tail: [u8; 4],
    pub(crate) tail_len: usize,
}

impl CurseForgeFingerprintHasher {
    pub(crate) fn new(normalized_size: u32) -> Self {
        Self {
            hash: 1 ^ normalized_size,
            tail: [0; 4],
            tail_len: 0,
        }
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if is_curseforge_fingerprint_whitespace(byte) {
                continue;
            }
            self.tail[self.tail_len] = byte;
            self.tail_len += 1;
            if self.tail_len == self.tail.len() {
                self.mix_word(u32::from_le_bytes(self.tail));
                self.tail_len = 0;
            }
        }
    }

    fn mix_word(&mut self, mut value: u32) {
        const M: u32 = 0x5bd1e995;
        const R: u32 = 24;
        value = value.wrapping_mul(M);
        value ^= value >> R;
        value = value.wrapping_mul(M);
        self.hash = self.hash.wrapping_mul(M);
        self.hash ^= value;
    }

    pub(crate) fn finish(mut self) -> u32 {
        const M: u32 = 0x5bd1e995;
        match self.tail[..self.tail_len] {
            [a, b, c] => {
                self.hash ^= (c as u32) << 16;
                self.hash ^= (b as u32) << 8;
                self.hash ^= a as u32;
                self.hash = self.hash.wrapping_mul(M);
            }
            [a, b] => {
                self.hash ^= (b as u32) << 8;
                self.hash ^= a as u32;
                self.hash = self.hash.wrapping_mul(M);
            }
            [a] => {
                self.hash ^= a as u32;
                self.hash = self.hash.wrapping_mul(M);
            }
            [] => {}
            _ => unreachable!(),
        }
        self.hash ^= self.hash >> 13;
        self.hash = self.hash.wrapping_mul(M);
        self.hash ^= self.hash >> 15;
        self.hash
    }
}

pub fn compute_fingerprint(data: &[u8]) -> u32 {
    let normalized_size = data
        .iter()
        .filter(|byte| !is_curseforge_fingerprint_whitespace(**byte))
        .count() as u32;
    let mut hasher = CurseForgeFingerprintHasher::new(normalized_size);
    hasher.update(data);
    hasher.finish()
}
