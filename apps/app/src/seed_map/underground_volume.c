#include "cubiomes_bridge.h"
#include "terrainnoise.h"
#include "carver.h"

#include <stdlib.h>
#include <string.h>

int32_t axolotl_seed_map_underground_volume(
    uint64_t seed, int32_t mc, int32_t flags, int32_t dimension,
    int32_t x, int32_t z, int32_t y, int32_t width, int32_t height,
    uint8_t *blocks, uint64_t length
) {
    if (mc < MC_1_14 || (dimension != DIM_OVERWORLD && dimension != DIM_NETHER) ||
        width < 16 || width > 64 || width % 16 || height < 8 || height > 96 || height % 8 ||
        x < -30000000 || x > 30000000 - width || z < -30000000 || z > 30000000 - width ||
        x % 16 || z % 16 || !blocks || length != (uint64_t)width * width * height) return -1;
    const int min_y = dimension == DIM_OVERWORLD && mc >= MC_1_18 ? -64 : 0;
    const int max_y = dimension == DIM_OVERWORLD ? (mc >= MC_1_18 ? 320 : 256) : 128;
    if (y < min_y || y > max_y - height || (y - min_y) % 8) return -1;
    TerrainNoise *terrain = calloc(1, sizeof(TerrainNoise));
    if (!terrain) return -2;
    setupTerrainNoise(terrain, mc, flags);
    initTerrainNoise(terrain, seed, dimension);
    generateRegion(terrain, x / 16, z / 16, width / 16, width / 16,
        (uint8_t (*)[])blocks, (y - min_y) / 8, (y + height - min_y) / 8, NULL, 0);

    for (int cx = x / 16; cx < (x + width) / 16; cx++) {
        for (int cz = z / 16; cz < (z + width) / 16; cz++) {
            int biomes[17][17];
            memset(biomes, 0, sizeof(biomes));
            if (mc < MC_1_18) {
                for (int rz = 0; rz < 17; rz++) for (int rx = 0; rx < 17; rx++)
                    biomes[rz][rx] = getBiomeAt(&terrain->g, 4, (cx + rx - 8) * 4, 0, (cz + rz - 8) * 4);
            }
            for (int layer = 0; layer < 2; layer++) {
                const int count = layer ? CANYON_CARVER_NUM : CAVE_CARVER_NUM;
                for (int type = 0; type < count; type++) {
                    Pos3List positions;
                    createPos3List(&positions, 1024);
                    if (layer) {
                        CanyonCarverConfig config;
                        if (getCanyonCarverConfig(type, mc, &config) && config.dim == dimension)
                            carveCanyon(seed, mc, cx, cz, config, type, biomes, &positions);
                    } else {
                        CaveCarverConfig config;
                        int biome = mc < MC_1_18 ? getBiomeAt(&terrain->g, 4, cx * 4, 0, cz * 4) : -1;
                        if (getCaveCarverConfig(type, mc, biome, &config) && config.dim == dimension)
                            carveCave(seed, mc, cx, cz, config, type, biomes, &positions);
                    }
                    for (int i = 0; i < positions.size; i++) {
                        const Pos3 p = positions.pos3s[i];
                        if (p.x < x || p.x >= x + width || p.z < z || p.z >= z + width || p.y < y || p.y >= y + height) continue;
                        blocks[((size_t)(p.x - x) * width + p.z - z) * height + p.y - y] = 0;
                    }
                    freePos3List(&positions);
                }
            }
        }
    }
    int *surfaces = NULL;
    if (dimension == DIM_OVERWORLD && y < 63) {
        surfaces = calloc((size_t)width * width, sizeof(int));
        if (!surfaces) { free(terrain); return -2; }
        generateRegion(terrain, x / 16, z / 16, width / 16, width / 16,
            NULL, 0, (max_y - min_y) / 8, surfaces, 1);
    }
    const int lava_level = dimension == DIM_NETHER ? 31 : mc >= MC_1_18 ? -54 : 10;
    for (int bx = 0; bx < width; bx++) for (int bz = 0; bz < width; bz++) {
        size_t column = (size_t)bx * width + bz;
        for (int by = 0; by < height; by++) {
            size_t index = column * height + by;
            if (blocks[index]) continue;
            if (y + by < lava_level) blocks[index] = 5;
            else if (surfaces && y + by < 63 && y + by >= surfaces[column]) blocks[index] = 4;
        }
    }
    free(surfaces);
    free(terrain);
    return 0;
}
