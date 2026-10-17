use std::{ffi::CStr, io, sync::Mutex};

use serde::Deserialize;

use super::{
    Dimension, Edition, TileRequest, parse_seed, resolve_java_version,
};

static STRUCTURE_COMPUTATION: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructureQuery {
    pub seed: String,
    pub edition: Edition,
    pub version: String,
    pub dimension: Dimension,
    pub x: i32,
    pub z: i32,
    pub feature_mask: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UndergroundRequest {
    pub tile: TileRequest,
    pub layers: u32,
}

struct NativeResult(*mut std::ffi::c_char);

impl Drop for NativeResult {
    fn drop(&mut self) {
        unsafe { super::axolotl_seed_map_free_result(self.0) };
    }
}

pub fn structure_details(
    query: StructureQuery,
    container: Option<u32>,
) -> io::Result<serde_json::Value> {
    let (mc, _) = resolve_java_version(&query.version)?;
    if query.x.abs_diff(0) > 30_000_000
        || query.z.abs_diff(0) > 30_000_000
        || query.feature_mask.count_ones() != 1
        || query.feature_mask & super::ALL_FEATURES == 0
        || container.is_some_and(|index| index >= 8192)
    {
        return Err(io::Error::other("The structure query is invalid."));
    }
    let supported = unsafe {
        super::axolotl_seed_map_capability(mc, 1, query.dimension.as_cubiomes())
    } as u32;
    if supported & query.feature_mask == 0 {
        return Err(io::Error::other(
            "This structure is not supported in the selected version and dimension.",
        ));
    }
    let _guard = STRUCTURE_COMPUTATION.lock().map_err(|_| {
        io::Error::other(
            "Structure calculation was interrupted. Retry this query.",
        )
    })?;
    let result = NativeResult(unsafe {
        super::axolotl_seed_map_structure_details(
            parse_seed(&query.seed),
            mc,
            query.edition.generator_flags(),
            query.dimension.as_cubiomes(),
            query.x,
            query.z,
            query.feature_mask,
            container.map_or(-1, |index| index as i32),
        )
    });
    if result.0.is_null() {
        return Err(io::Error::other(
            "The engine could not allocate the structure result.",
        ));
    }
    let bytes = unsafe { CStr::from_ptr(result.0) }.to_bytes();
    serde_json::from_slice(bytes).map_err(io::Error::other)
}

pub fn underground_tile(request: UndergroundRequest) -> io::Result<Vec<u8>> {
    let tile = request.tile;
    let (mc, code) = resolve_java_version(&tile.version)?;
    super::validate_tile_request(&tile)?;
    let min_y =
        if matches!(tile.dimension, Dimension::Overworld) && code >= 11800 {
            -64
        } else {
            0
        };
    let max_y = if matches!(tile.dimension, Dimension::Overworld) {
        if code >= 11800 { 319 } else { 255 }
    } else {
        127
    };
    let y = tile.elevation.unwrap_or(16);
    if code < 11300
        || (request.layers & 1 != 0 && code < 11400)
        || tile.width > 64
        || tile.height > 64
        || tile.width % 16 != 0
        || tile.height % 16 != 0
        || !matches!(tile.scale, 1 | 4)
        || tile.x % 16 != 0
        || tile.z % 16 != 0
        || y < min_y
        || y > max_y
        || request.layers & !7 != 0
    {
        return Err(io::Error::other(
            "The underground tile bounds, height or layers are invalid.",
        ));
    }
    let supported = unsafe {
        super::axolotl_seed_map_capability(mc, 3, tile.dimension.as_cubiomes())
    } as u32;
    if request.layers & !supported != 0 {
        return Err(io::Error::other(
            "These underground layers are not supported in the selected version and dimension.",
        ));
    }
    let mut rgba = vec![0; tile.width as usize * tile.height as usize * 4];
    let status = unsafe {
        super::axolotl_seed_map_underground(
            parse_seed(&tile.seed),
            mc,
            tile.edition.generator_flags(),
            tile.dimension.as_cubiomes(),
            tile.x,
            tile.z,
            tile.scale,
            tile.width,
            tile.height,
            y,
            request.layers,
            rgba.as_mut_ptr(),
            rgba.len() as u64,
        )
    };
    if status != 0 {
        return Err(io::Error::other(format!(
            "Underground generation failed ({status})."
        )));
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn query(
        seed: &str,
        version: &str,
        x: i32,
        z: i32,
        feature_mask: u32,
    ) -> StructureQuery {
        StructureQuery {
            seed: seed.into(),
            edition: Edition::Java,
            version: version.into(),
            dimension: Dimension::Overworld,
            x,
            z,
            feature_mask,
        }
    }
    #[test]
    fn abandoned_camp_matches_upstream_vanilla_fixture() {
        let query = query(
            "-3829811542736183482",
            "26.3",
            224,
            -384,
            super::super::FEATURE_ABANDONED_CAMP,
        );
        let result = structure_details(query.clone(), None).unwrap();
        assert_eq!(result["supported"], true);
        assert_eq!(result["pieces"].as_array().unwrap().len(), 2);
        assert!(
            result["pieces"][0]["name"]
                .as_str()
                .unwrap()
                .ends_with("_9")
        );
        assert_eq!(
            result["containers"][0]["table"],
            "barrels/abandoned_camp_barrel"
        );
        let loot = structure_details(query, Some(0)).unwrap();
        assert_eq!(loot["supported"], true);
        assert!(!loot["items"].as_array().unwrap().is_empty());
    }
    #[test]
    fn outpost_loot_matches_upstream_enchantment_fixture() {
        let result = structure_details(
            query(
                "-7594343816690189614",
                "1.21.9",
                1344,
                -9872,
                super::super::FEATURE_OUTPOST,
            ),
            Some(0),
        )
        .unwrap();
        assert!(result["items"].as_array().unwrap().iter().any(
            |item| item["id"] == "minecraft:book"
                && item["enchantments"].as_array().unwrap().iter().any(
                    |enchantment| {
                        enchantment["id"]
                            .as_str()
                            .is_some_and(|id| id.ends_with("fortune"))
                            && enchantment["level"] == 1
                    }
                )
        ));
    }
    #[test]
    fn underground_slices_are_bounded_and_deterministic() {
        let request = UndergroundRequest {
            tile: TileRequest {
                seed: "12345".into(),
                edition: Edition::Java,
                version: "26.3".into(),
                dimension: Dimension::Overworld,
                x: -16,
                z: 0,
                scale: 1,
                width: 16,
                height: 16,
                elevation: Some(-32),
                terrain: false,
                contours: false,
                highlight_biomes: None,
            },
            layers: 7,
        };
        let image = underground_tile(request.clone()).unwrap();
        assert_eq!(image.len(), 16 * 16 * 4);
        assert!(image.chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert_eq!(image, underground_tile(request.clone()).unwrap());
        let mut legacy = request.clone();
        legacy.tile.version = "1.13.2".into();
        legacy.tile.elevation = Some(32);
        legacy.layers = 6;
        assert_eq!(underground_tile(legacy).unwrap().len(), 16 * 16 * 4);
        let mut invalid = request;
        invalid.tile.elevation = Some(320);
        assert!(underground_tile(invalid).is_err());
    }
}
