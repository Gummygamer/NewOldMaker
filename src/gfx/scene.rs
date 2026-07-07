//! Helpers that turn game/editor state into renderer `FrameInput` pieces.

use glam::Vec3;

use crate::core::data::{MapAmbience, MapData, Prop};
use crate::gfx::camera::OrbitCamera;
use crate::gfx::mesh::{tile_top_y, SpriteInstance, SPRITE_EMISSIVE, SPRITE_HORIZONTAL, SPRITE_UNLIT};
use crate::gfx::pixelart::{Atlas, UvRect, CHAR_FRAMES};
use crate::gfx::renderer::{FrameInput, LightSpec, PostSettings};

pub const CHAR_SIZE: [f32; 2] = [0.75, 1.125];
pub const PROP_SIZE: [f32; 2] = [1.0, 1.5];

pub fn sprite(pos: Vec3, size: [f32; 2], uv: UvRect, tint: [f32; 4], flags: u32) -> SpriteInstance {
    SpriteInstance {
        pos: [pos.x, pos.y, pos.z],
        size,
        uv0: [uv.u0, uv.v0],
        uv1: [uv.u1, uv.v1],
        tint,
        flags,
        _pad: [0; 3],
    }
}

/// Feet-anchored character sprite + its shadow.
pub fn char_sprites(
    atlas: &Atlas,
    sheet: usize,
    dir: u32,
    frame: u32,
    pos: Vec3,
    tint: [f32; 4],
    out_cutout: &mut Vec<SpriteInstance>,
    out_blend: &mut Vec<SpriteInstance>,
) {
    let sheet = sheet % atlas.chars.len();
    let uv = atlas.chars[sheet][((dir % 4) * CHAR_FRAMES + (frame % CHAR_FRAMES)) as usize];
    out_cutout.push(sprite(pos + Vec3::Y * 0.02, CHAR_SIZE, uv, tint, 0));
    out_blend.push(sprite(
        pos + Vec3::Y * 0.015,
        [0.55, 0.32],
        atlas.shadow,
        [1.0, 1.0, 1.0, 0.8],
        SPRITE_HORIZONTAL | SPRITE_UNLIT,
    ));
}

/// All prop billboards on a map, plus the point lights they emit.
pub fn map_prop_sprites(
    map: &MapData,
    atlas: &Atlas,
    out_cutout: &mut Vec<SpriteInstance>,
    out_lights: &mut Vec<LightSpec>,
) {
    for ty in 0..map.height as i32 {
        for tx in 0..map.width as i32 {
            let tile = map.tile(tx, ty);
            if tile.prop == 0 {
                continue;
            }
            let prop = Prop::from_u8(tile.prop);
            let top = tile_top_y(map, tx, ty);
            let pos = Vec3::new(tx as f32 + 0.5, top, ty as f32 + 0.5);
            let flags = if prop.light().is_some() { SPRITE_EMISSIVE } else { 0 };
            out_cutout.push(sprite(pos, PROP_SIZE, atlas.props[tile.prop as usize], [1.0; 4], flags));
            if let Some(color) = prop.light() {
                out_lights.push(LightSpec {
                    pos: pos + Vec3::Y * 1.0,
                    radius: 4.5,
                    color: [color[0] * 1.6, color[1] * 1.6, color[2] * 1.6],
                });
            }
        }
    }
}

/// Assemble a `FrameInput` from a map + camera + already-collected sprites.
#[allow(clippy::too_many_arguments)]
pub fn frame_input(
    map: std::sync::Arc<MapData>,
    map_revision: u64,
    camera: &OrbitCamera,
    viewport_px: [u32; 2],
    time: f32,
    lights: Vec<LightSpec>,
    sprites_cutout: Vec<SpriteInstance>,
    sprites_blend: Vec<SpriteInstance>,
    post: PostSettings,
) -> FrameInput {
    let amb: MapAmbience = map.ambience.clone();
    let aspect = viewport_px[0] as f32 / viewport_px[1].max(1) as f32;
    let sun_scale = 1.0 - amb.darkness * 0.92;
    FrameInput {
        view_proj: camera.view_proj(aspect),
        cam_pos: camera.eye(),
        cam_right: camera.billboard_right(),
        sun_dir: Vec3::new(-0.45, -1.0, -0.35).normalize(),
        sun_color: [
            amb.sun_color[0] * sun_scale,
            amb.sun_color[1] * sun_scale,
            amb.sun_color[2] * sun_scale,
        ],
        ambient: amb.ambient_color,
        fog_color: amb.fog_color,
        fog_density: amb.fog_density,
        darkness: amb.darkness,
        time,
        lights,
        sprites_cutout,
        sprites_blend,
        map,
        map_revision,
        post: PostSettings { bloom_strength: amb.bloom_strength, ..post },
        viewport_px,
    }
}
