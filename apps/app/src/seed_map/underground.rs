use super::{
    Dimension, Edition,
    ores::{OreKind, OreScanRequest, scan_ores, scan_ores_with_budget},
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
    #[serde(default)]
    pub chunk: bool,
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
        || request.x
            > if request.chunk {
                29_999_984
            } else {
                29_999_936
            }
        || request.z < -30_000_000
        || request.z
            > if request.chunk {
                29_999_984
            } else {
                29_999_936
            }
        || request.y < min_y
        || if request.chunk {
            request.y != min_y
        } else {
            request.y > max_y - 64 || (request.y - min_y) % 8 != 0
        }
    {
        return Err(io::Error::other(
            "Invalid underground region. A 64 × 64 × 64 block region within Java 1.14+ world bounds is required.",
        ));
    }
    if request.chunk {
        return generate_chunk(&request, mc, code);
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

/// Schema 3 contains an 18-wide X/Z volume across the complete build height,
/// then i16 surface Y coordinates. Only the inner 16 columns render.
fn generate_chunk(
    request: &VolumeRequest,
    mc: i32,
    code: i32,
) -> io::Result<Vec<u8>> {
    let source_x = (request.x - 16).clamp(-30_000_000, 29_999_952);
    let source_z = (request.z - 16).clamp(-30_000_000, 29_999_952);
    let min_y =
        if matches!(request.dimension, Dimension::Overworld) && code >= 11800 {
            -64
        } else {
            0
        };
    let max_y = match request.dimension {
        Dimension::Nether => 128,
        Dimension::Overworld if code >= 11800 => 320,
        _ => 256,
    };
    let height = max_y - min_y;
    let mut source = vec![0_u8; 48 * 48 * height as usize];
    let mut heights = vec![0_i32; 48 * 48];
    let status = unsafe {
        super::axolotl_seed_map_underground_volume_surface(
            parse_seed(&request.seed),
            mc,
            request.edition.generator_flags(),
            request.dimension.as_cubiomes(),
            source_x,
            source_z,
            request.y,
            48,
            height,
            source.as_mut_ptr(),
            source.len() as u64,
            heights.as_mut_ptr(),
        )
    };
    if status != 0 {
        return Err(io::Error::other(format!(
            "Underground chunk generation failed ({status})."
        )));
    }
    check_active(request.id)?;
    let mut data = vec![1_u8; 18 * 18 * height as usize];
    let mut surface = vec![i16::MIN; 18 * 18];
    for x in 0..18_i32 {
        for z in 0..18_i32 {
            let sx = request.x - 1 + x - source_x;
            let sz = request.z - 1 + z - source_z;
            if !(0..48).contains(&sx) || !(0..48).contains(&sz) {
                continue;
            }
            let surface_y = heights[(sx * 48 + sz) as usize] - 1;
            if surface_y >= min_y {
                surface[(x * 18 + z) as usize] = surface_y as i16;
            }
            for y in 0..height {
                let block = source[((sx * 48 + sz) * height + y) as usize];
                data[((x * 18 + z) * height + y) as usize] = match block {
                    1 if matches!(request.dimension, Dimension::Nether) => 3,
                    1 if code >= 11800 && request.y + y < 0 => 2,
                    value => value,
                };
            }
        }
    }
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
    let mut truncated = false;
    for cx in -1..=1 {
        for cz in -1..=1 {
            check_active(request.id)?;
            let cx_world = request.x / 16 + cx;
            let cz_world = request.z / 16 + cz;
            if cx_world.abs_diff(0) > 1_875_000
                || cz_world.abs_diff(0) > 1_875_000
            {
                continue;
            }
            let ores: Vec<_> = kinds
                .iter()
                .copied()
                .filter(|kind| {
                    (cx == 0 && cz == 0)
                        || !matches!(
                            kind,
                            OreKind::IronVein | OreKind::CopperVein
                        )
                })
                .collect();
            let capacity = 16_384 * ores.len();
            for result in scan_ores_with_budget(
                OreScanRequest {
                    seed: request.seed.clone(),
                    edition: request.edition,
                    version: request.version.clone(),
                    dimension: request.dimension,
                    ores,
                    chunks: vec![cx_world, cz_world],
                },
                capacity,
            )? {
                truncated |= result.truncated;
                for hit in result.hits {
                    let x = hit.x - request.x + 1;
                    let z = hit.z - request.z + 1;
                    let y = hit.y - request.y;
                    if !(0..18).contains(&x)
                        || !(0..18).contains(&z)
                        || !(0..height).contains(&y)
                    {
                        continue;
                    }
                    let index = ((x * 18 + z) * height + y) as usize;
                    let host = data[index];
                    if (1..=3).contains(&host) {
                        data[index] =
                            16 + hit.ore as u8 + if host == 2 { 64 } else { 0 };
                    }
                }
            }
        }
    }
    check_active(request.id)?;
    let mut response = Vec::with_capacity(16 + data.len() + surface.len() * 2);
    for value in [3_u32, u32::from(truncated), 18, height as u32] {
        response.extend_from_slice(&value.to_le_bytes());
    }
    response.extend(data);
    for value in surface {
        response.extend_from_slice(&value.to_le_bytes());
    }
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
            chunk: false,
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
                *id <= 6 || (16..=28).contains(id) || (80..=92).contains(id)
            );
        }
        let mut chunk = request.clone();
        chunk.chunk = true;
        let ore_request = || OreScanRequest {
            seed: request.seed.clone(),
            edition: request.edition,
            version: request.version.clone(),
            dimension: request.dimension,
            ores: vec![OreKind::Diamond, OreKind::Iron],
            chunks: vec![0, 0],
        };
        let small = scan_ores_with_budget(ore_request(), 2).unwrap();
        let expanded = scan_ores_with_budget(ore_request(), 32_768).unwrap();
        assert!(small[0].truncated);
        assert!(expanded[0].hits.len() > small[0].hits.len());
        assert!(!expanded[0].truncated);
        let bytes = generate_volume(chunk.clone()).unwrap();
        assert_eq!(bytes.len(), 16 + 18 * 18 * 384 + 18 * 18 * 2);
        assert_eq!(&bytes[..4], &3_u32.to_le_bytes());
        let heights = &bytes[16 + 18 * 18 * 384..];
        assert!(
            heights
                .chunks_exact(2)
                .any(|value| i16::from_le_bytes([value[0], value[1]]) >= 0)
        );
        chunk.x = -30_000_000;
        chunk.z = -30_000_000;
        assert!(generate_volume(chunk).is_ok());
        cancel(request.id);
        assert!(generate_volume(request).is_err());
    }
}
