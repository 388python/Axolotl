use super::{
    Dimension, Edition,
    ores::{OreKind, OreScanRequest, scan_ores},
    parse_seed, resolve_java_version,
};
use serde::Deserialize;
use std::{
    io,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

static ACTIVE_REQUEST: AtomicU64 = AtomicU64::new(0);
static GENERATOR: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeRequest {
    pub id: u64,
    pub seed: String,
    pub edition: Edition,
    pub version: String,
    pub dimension: Dimension,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

pub fn begin(id: u64) {
    ACTIVE_REQUEST.store(id, Ordering::Relaxed);
}

pub fn cancel(id: u64) {
    let _ = ACTIVE_REQUEST.compare_exchange(
        id,
        0,
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
}

fn check_active(id: u64) -> io::Result<()> {
    if id == 0 || ACTIVE_REQUEST.load(Ordering::Relaxed) != id {
        return Err(io::Error::other("Underground generation canceled."));
    }
    Ok(())
}

/// Schema 1: four little-endian u32 fields (schema, flags, width, height),
/// followed by u8 material IDs in X/Z/Y order. Flags: bit 0 = truncated ores.
pub fn generate_volume(request: VolumeRequest) -> io::Result<Vec<u8>> {
    check_active(request.id)?;
    let _guard = GENERATOR
        .lock()
        .map_err(|error| io::Error::other(error.to_string()))?;
    check_active(request.id)?;
    let (mc, code) = resolve_java_version(&request.version)?;
    let min_y =
        if matches!(request.dimension, Dimension::Overworld) && code >= 11800 {
            -64
        } else {
            0
        };
    let max_y = match request.dimension {
        Dimension::Overworld => {
            if code >= 11800 {
                320
            } else {
                256
            }
        }
        Dimension::Nether => 128,
        Dimension::End => {
            return Err(io::Error::other(
                "The End has no supported underground volume.",
            ));
        }
    };
    if code < 11400
        || request.x % 16 != 0
        || request.z % 16 != 0
        || request.x < -30_000_000
        || request.x > 29_999_936
        || request.z < -30_000_000
        || request.z > 29_999_936
        || request.y < min_y
        || request.y > max_y - 64
        || (request.y - min_y) % 8 != 0
    {
        return Err(io::Error::other(
            "Invalid underground region. A 64 × 64 × 64 block region within Java 1.14+ world bounds is required.",
        ));
    }
    let mut data = vec![0_u8; 64 * 64 * 64];
    let mut truncated = false;
    let mut chunk = vec![0_u8; 16 * 16 * 64];
    let kinds = if matches!(request.dimension, Dimension::Nether) {
        vec![OreKind::Netherite, OreKind::Quartz, OreKind::NetherGold]
    } else {
        vec![
            OreKind::Diamond,
            OreKind::Iron,
            OreKind::IronVein,
            OreKind::Copper,
            OreKind::CopperVein,
            OreKind::Gold,
            OreKind::Redstone,
            OreKind::Lapis,
            OreKind::Coal,
            OreKind::Emerald,
        ]
    };
    for cx in 0..4 {
        for cz in 0..4 {
            check_active(request.id)?;
            let x = request.x + cx * 16;
            let z = request.z + cz * 16;
            let status = unsafe {
                super::axolotl_seed_map_underground_volume(
                    parse_seed(&request.seed),
                    mc,
                    request.edition.generator_flags(),
                    request.dimension.as_cubiomes(),
                    x,
                    z,
                    request.y,
                    16,
                    64,
                    chunk.as_mut_ptr(),
                    chunk.len() as u64,
                )
            };
            if status != 0 {
                return Err(io::Error::other(format!(
                    "Underground volume generation failed ({status})."
                )));
            }
            for bx in 0..16 {
                for bz in 0..16 {
                    for by in 0..64 {
                        let source = ((bx * 16 + bz) * 64 + by) as usize;
                        let target = (((cx * 16 + bx) * 64 + cz * 16 + bz) * 64
                            + by) as usize;
                        data[target] = match chunk[source] {
                            1 if matches!(
                                request.dimension,
                                Dimension::Nether
                            ) =>
                            {
                                3
                            }
                            1 if code >= 11800 && request.y + by < 0 => 2,
                            value => value,
                        };
                    }
                }
            }
        }
    }
    for cx in -1..5 {
        for cz in -1..5 {
            check_active(request.id)?;
            let source_x = request.x / 16 + cx;
            let source_z = request.z / 16 + cz;
            if source_x.abs_diff(0) > 1_875_000
                || source_z.abs_diff(0) > 1_875_000
            {
                continue;
            }
            let halo = !(0..4).contains(&cx) || !(0..4).contains(&cz);
            let scan_kinds = kinds
                .iter()
                .copied()
                .filter(|kind| {
                    !halo
                        || !matches!(
                            kind,
                            OreKind::IronVein | OreKind::CopperVein
                        )
                })
                .collect();
            let results = scan_ores(OreScanRequest {
                seed: request.seed.clone(),
                edition: request.edition,
                version: request.version.clone(),
                dimension: request.dimension,
                ores: scan_kinds,
                chunks: vec![source_x, source_z],
            })?;
            for result in results {
                truncated |= result.truncated;
                for hit in result.hits {
                    if hit.x < request.x
                        || hit.x >= request.x + 64
                        || hit.z < request.z
                        || hit.z >= request.z + 64
                        || hit.y < request.y
                        || hit.y >= request.y + 64
                    {
                        continue;
                    }
                    let index = (((hit.x - request.x) * 64 + hit.z - request.z)
                        * 64
                        + hit.y
                        - request.y) as usize;
                    let host = data[index];
                    if !(1..=3).contains(&host) {
                        continue;
                    }
                    data[index] =
                        16 + hit.ore as u8 + if host == 2 { 64 } else { 0 };
                }
            }
        }
    }
    check_active(request.id)?;
    let mut response = Vec::with_capacity(data.len() + 16);
    for value in [1_u32, u32::from(truncated), 64, 64] {
        response.extend_from_slice(&value.to_le_bytes());
    }
    response.extend(data);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_bounds_cancellation_and_materials() {
        let request = VolumeRequest {
            id: 99,
            seed: "12345".into(),
            edition: Edition::Java,
            version: "26.3".into(),
            dimension: Dimension::Overworld,
            x: -32,
            y: -64,
            z: -32,
        };
        begin(request.id);
        let mut invalid = request.clone();
        invalid.y = i32::MAX;
        assert!(generate_volume(invalid).is_err());
        let bytes = generate_volume(request.clone()).unwrap();
        assert_eq!(bytes.len(), 16 + 64 * 64 * 64);
        assert_eq!(&bytes[..4], &1_u32.to_le_bytes());
        assert!(bytes[16..].contains(&0));
        assert!(bytes[16..].contains(&2));
        for (index, id) in bytes[16..].iter().enumerate() {
            let y = request.y + (index % 64) as i32;
            if *id == 5 {
                assert!(y < -54);
            }
            if *id == 4 {
                assert!(y < 63);
            }
            assert!(
                *id <= 5 || (16..=28).contains(id) || (80..=92).contains(id)
            );
        }
        cancel(request.id);
        assert!(generate_volume(request).is_err());
    }
}
