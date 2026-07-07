//! CPU-side geometry: terrain mesh building and sprite instance layout.

use crate::core::data::{MapData, Terrain};
use crate::gfx::pixelart::Atlas;
use bytemuck::{Pod, Zeroable};

pub const HEIGHT_STEP: f32 = 0.5;
/// How far liquid surfaces sit below their tile top.
pub const LIQUID_DROP: f32 = 0.22;

pub const FLAG_LIQUID: u32 = 1;
pub const FLAG_EMISSIVE: u32 = 2;

pub const SPRITE_HORIZONTAL: u32 = 1;
pub const SPRITE_UNLIT: u32 = 2;
pub const SPRITE_EMISSIVE: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct TerrainVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct SpriteInstance {
    pub pos: [f32; 3],
    pub size: [f32; 2],
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    pub tint: [f32; 4],
    pub flags: u32,
    pub _pad: [u32; 3],
}

/// World-space Y of the walkable surface of a tile.
pub fn tile_top_y(map: &MapData, x: i32, y: i32) -> f32 {
    let t = map.tile(x, y);
    let terr = Terrain::from_u8(t.terrain);
    let mut top = t.height as f32 * HEIGHT_STEP;
    if terr.liquid() {
        top -= LIQUID_DROP;
    }
    top
}

pub fn build_terrain_mesh(map: &MapData, atlas: &Atlas) -> (Vec<TerrainVertex>, Vec<u32>) {
    let mut verts: Vec<TerrainVertex> = Vec::with_capacity((map.width * map.height * 8) as usize);
    let mut idx: Vec<u32> = Vec::with_capacity((map.width * map.height * 12) as usize);

    let quad = |v: [TerrainVertex; 4], out_idx: &mut Vec<u32>, out_v: &mut Vec<TerrainVertex>| {
        let base = out_v.len() as u32;
        out_v.extend_from_slice(&v);
        out_idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };

    for ty in 0..map.height as i32 {
        for tx in 0..map.width as i32 {
            let tile = map.tile(tx, ty);
            let terr = Terrain::from_u8(tile.terrain);
            let top = tile_top_y(map, tx, ty);
            let r = atlas.terrain_top[tile.terrain as usize % atlas.terrain_top.len()];
            let mut flags = 0u32;
            if terr.liquid() {
                flags |= FLAG_LIQUID;
            }
            if terr == Terrain::Lava {
                flags |= FLAG_EMISSIVE;
            }
            let (x0, z0, x1, z1) = (tx as f32, ty as f32, tx as f32 + 1.0, ty as f32 + 1.0);
            // Top face.
            quad(
                [
                    TerrainVertex { pos: [x0, top, z0], normal: [0.0, 1.0, 0.0], uv: [r.u0, r.v0], flags },
                    TerrainVertex { pos: [x1, top, z0], normal: [0.0, 1.0, 0.0], uv: [r.u1, r.v0], flags },
                    TerrainVertex { pos: [x1, top, z1], normal: [0.0, 1.0, 0.0], uv: [r.u1, r.v1], flags },
                    TerrainVertex { pos: [x0, top, z1], normal: [0.0, 1.0, 0.0], uv: [r.u0, r.v1], flags },
                ],
                &mut idx,
                &mut verts,
            );

            // Exposed side faces down to each lower neighbor.
            let side_uv = atlas.terrain_side[tile.terrain as usize % atlas.terrain_side.len()];
            // (dx, dy, normal, corner order along the edge)
            let sides: [(i32, i32, [f32; 3], [f32; 2], [f32; 2]); 4] = [
                (0, 1, [0.0, 0.0, 1.0], [x0, z1], [x1, z1]),  // south
                (0, -1, [0.0, 0.0, -1.0], [x1, z0], [x0, z0]), // north
                (1, 0, [1.0, 0.0, 0.0], [x1, z1], [x1, z0]),  // east
                (-1, 0, [-1.0, 0.0, 0.0], [x0, z0], [x0, z1]), // west
            ];
            for (dx, dy, normal, ca, cb) in sides {
                let neighbor_top = if map.in_bounds(tx + dx, ty + dy) {
                    tile_top_y(map, tx + dx, ty + dy)
                } else {
                    -1.0 // map border: skirt down a bit
                };
                if neighbor_top >= top - 0.001 {
                    continue;
                }
                // Emit one band per HEIGHT_STEP so the strata texture repeats.
                let mut y1 = top;
                while y1 > neighbor_top + 0.001 {
                    let y0 = (y1 - HEIGHT_STEP).max(neighbor_top);
                    let vfrac = (y1 - y0) / HEIGHT_STEP;
                    let v_hi = side_uv.v0;
                    let v_lo = side_uv.v0 + (side_uv.v1 - side_uv.v0) * vfrac;
                    quad(
                        [
                            TerrainVertex { pos: [ca[0], y1, ca[1]], normal, uv: [side_uv.u0, v_hi], flags: 0 },
                            TerrainVertex { pos: [cb[0], y1, cb[1]], normal, uv: [side_uv.u1, v_hi], flags: 0 },
                            TerrainVertex { pos: [cb[0], y0, cb[1]], normal, uv: [side_uv.u1, v_lo], flags: 0 },
                            TerrainVertex { pos: [ca[0], y0, ca[1]], normal, uv: [side_uv.u0, v_lo], flags: 0 },
                        ],
                        &mut idx,
                        &mut verts,
                    );
                    y1 = y0;
                }
            }
        }
    }
    (verts, idx)
}

/// March a screen ray against the heightfield; returns the tile hit, if any.
pub fn pick_tile(map: &MapData, origin: glam::Vec3, dir: glam::Vec3) -> Option<(i32, i32)> {
    let mut t = 0.0f32;
    let step = 0.05f32;
    while t < 200.0 {
        let p = origin + dir * t;
        let tx = p.x.floor() as i32;
        let ty = p.z.floor() as i32;
        if map.in_bounds(tx, ty) {
            let top = tile_top_y(map, tx, ty);
            if p.y <= top {
                return Some((tx, ty));
            }
        } else if p.y < -2.0 {
            return None;
        }
        t += step;
    }
    None
}
