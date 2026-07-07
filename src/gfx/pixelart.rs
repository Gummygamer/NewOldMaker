//! Procedurally generated pixel-art assets. Everything the engine displays is
//! drawn at startup into a single RGBA atlas — no asset files required.

use crate::core::data::{Prop, Terrain, PROP_COUNT, TERRAIN_COUNT};

pub const TILE_PX: u32 = 32;
pub const CHAR_W: u32 = 16;
pub const CHAR_H: u32 = 24;
pub const PROP_W: u32 = 32;
pub const PROP_H: u32 = 48;
pub const ENEMY_PX: u32 = 48;
pub const CHAR_SHEETS: usize = 8;
pub const ENEMY_SPRITES: usize = 4;
/// Walk animation frames per direction.
pub const CHAR_FRAMES: u32 = 3;
/// Directions in sheet row order: down, left, right, up.
pub const CHAR_DIRS: u32 = 4;

pub const ATLAS_W: u32 = 1024;
pub const ATLAS_H: u32 = 512;

#[derive(Clone, Copy, Debug, Default)]
pub struct UvRect {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

pub struct Atlas {
    pub pixels: Vec<u8>, // RGBA8, ATLAS_W × ATLAS_H
    pub terrain_top: [UvRect; TERRAIN_COUNT],
    pub terrain_side: [UvRect; TERRAIN_COUNT],
    pub props: [UvRect; PROP_COUNT],
    /// [sheet][dir * CHAR_FRAMES + frame]
    pub chars: Vec<[UvRect; (CHAR_DIRS * CHAR_FRAMES) as usize]>,
    pub enemies: [UvRect; ENEMY_SPRITES],
    pub shadow: UvRect,
    pub white: UvRect,
}

// ---------------------------------------------------------------------------
// Tiny deterministic PRNG + pixmap
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 32) as u32
    }
    fn range(&mut self, n: u32) -> u32 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, percent: u32) -> bool {
        self.range(100) < percent
    }
}

type Color = [u8; 4];

fn shade(c: Color, f: f32) -> Color {
    [
        (c[0] as f32 * f).clamp(0.0, 255.0) as u8,
        (c[1] as f32 * f).clamp(0.0, 255.0) as u8,
        (c[2] as f32 * f).clamp(0.0, 255.0) as u8,
        c[3],
    ]
}

struct Painter<'a> {
    atlas: &'a mut Vec<u8>,
    ox: u32,
    oy: u32,
    w: u32,
    h: u32,
}

impl Painter<'_> {
    fn px(&mut self, x: i32, y: i32, c: Color) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 || c[3] == 0 {
            return;
        }
        let idx = (((self.oy + y as u32) * ATLAS_W + self.ox + x as u32) * 4) as usize;
        self.atlas[idx..idx + 4].copy_from_slice(&c);
    }
    fn rect(&mut self, x0: i32, y0: i32, w: i32, h: i32, c: Color) {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                self.px(x, y, c);
            }
        }
    }
    fn fill_noise(&mut self, base: Color, rng: &mut Rng, amount: f32) {
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let f = 1.0 + (rng.range(200) as f32 / 100.0 - 1.0) * amount;
                self.px(x, y, shade(base, f));
            }
        }
    }
    fn disc(&mut self, cx: i32, cy: i32, r: i32, c: Color) {
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                let dx = x - cx;
                let dy = y - cy;
                if dx * dx + dy * dy <= r * r {
                    self.px(x, y, c);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Atlas builder: simple shelf packer
// ---------------------------------------------------------------------------

struct Packer {
    x: u32,
    y: u32,
    row_h: u32,
}

impl Packer {
    fn alloc(&mut self, w: u32, h: u32) -> (u32, u32) {
        // +2px gutter against bleeding.
        if self.x + w + 2 > ATLAS_W {
            self.x = 0;
            self.y += self.row_h + 2;
            self.row_h = 0;
        }
        let pos = (self.x, self.y);
        self.x += w + 2;
        self.row_h = self.row_h.max(h);
        assert!(self.y + h <= ATLAS_H, "atlas overflow");
        pos
    }
}

fn uv(x: u32, y: u32, w: u32, h: u32) -> UvRect {
    UvRect {
        u0: x as f32 / ATLAS_W as f32,
        v0: y as f32 / ATLAS_H as f32,
        u1: (x + w) as f32 / ATLAS_W as f32,
        v1: (y + h) as f32 / ATLAS_H as f32,
    }
}

pub fn build_atlas() -> Atlas {
    let mut pixels = vec![0u8; (ATLAS_W * ATLAS_H * 4) as usize];
    let mut packer = Packer { x: 0, y: 0, row_h: 0 };
    let mut terrain_top = [UvRect::default(); TERRAIN_COUNT];
    let mut terrain_side = [UvRect::default(); TERRAIN_COUNT];
    let mut props = [UvRect::default(); PROP_COUNT];
    let mut enemies = [UvRect::default(); ENEMY_SPRITES];

    for (i, t) in crate::core::data::ALL_TERRAINS.iter().enumerate() {
        let (x, y) = packer.alloc(TILE_PX, TILE_PX);
        let mut p = Painter { atlas: &mut pixels, ox: x, oy: y, w: TILE_PX, h: TILE_PX };
        draw_terrain_top(&mut p, *t, i as u64);
        terrain_top[i] = uv(x, y, TILE_PX, TILE_PX);

        let (x, y) = packer.alloc(TILE_PX, TILE_PX);
        let mut p = Painter { atlas: &mut pixels, ox: x, oy: y, w: TILE_PX, h: TILE_PX };
        draw_terrain_side(&mut p, *t, i as u64 + 100);
        terrain_side[i] = uv(x, y, TILE_PX, TILE_PX);
    }

    for (i, prop) in crate::core::data::ALL_PROPS.iter().enumerate() {
        let (x, y) = packer.alloc(PROP_W, PROP_H);
        let mut p = Painter { atlas: &mut pixels, ox: x, oy: y, w: PROP_W, h: PROP_H };
        draw_prop(&mut p, *prop, i as u64 + 200);
        props[i] = uv(x, y, PROP_W, PROP_H);
    }

    let mut chars = Vec::with_capacity(CHAR_SHEETS);
    for sheet in 0..CHAR_SHEETS {
        let mut frames = [UvRect::default(); (CHAR_DIRS * CHAR_FRAMES) as usize];
        for dir in 0..CHAR_DIRS {
            for frame in 0..CHAR_FRAMES {
                let (x, y) = packer.alloc(CHAR_W, CHAR_H);
                let mut p = Painter { atlas: &mut pixels, ox: x, oy: y, w: CHAR_W, h: CHAR_H };
                draw_character(&mut p, sheet, dir, frame);
                frames[(dir * CHAR_FRAMES + frame) as usize] = uv(x, y, CHAR_W, CHAR_H);
            }
        }
        chars.push(frames);
    }

    for i in 0..ENEMY_SPRITES {
        let (x, y) = packer.alloc(ENEMY_PX, ENEMY_PX);
        let mut p = Painter { atlas: &mut pixels, ox: x, oy: y, w: ENEMY_PX, h: ENEMY_PX };
        draw_enemy(&mut p, i, i as u64 + 300);
        enemies[i] = uv(x, y, ENEMY_PX, ENEMY_PX);
    }

    // Soft round shadow blob.
    let (sx, sy) = packer.alloc(16, 8);
    {
        let mut p = Painter { atlas: &mut pixels, ox: sx, oy: sy, w: 16, h: 8 };
        for y in 0..8i32 {
            for x in 0..16i32 {
                let dx = (x as f32 - 7.5) / 7.5;
                let dy = (y as f32 - 3.5) / 3.5;
                let d = dx * dx + dy * dy;
                if d < 1.0 {
                    let a = ((1.0 - d) * 140.0) as u8;
                    p.px(x, y, [8, 8, 16, a]);
                }
            }
        }
    }
    let shadow = uv(sx, sy, 16, 8);

    let (wx, wy) = packer.alloc(4, 4);
    {
        let mut p = Painter { atlas: &mut pixels, ox: wx, oy: wy, w: 4, h: 4 };
        p.rect(0, 0, 4, 4, [255, 255, 255, 255]);
    }
    let white = uv(wx + 1, wy + 1, 2, 2);

    Atlas { pixels, terrain_top, terrain_side, props, chars, enemies, shadow, white }
}

// ---------------------------------------------------------------------------
// Terrain
// ---------------------------------------------------------------------------

fn terrain_base(t: Terrain) -> Color {
    match t {
        Terrain::Grass => [88, 148, 68, 255],
        Terrain::Dirt => [124, 94, 62, 255],
        Terrain::Stone => [116, 116, 124, 255],
        Terrain::Sand => [212, 188, 128, 255],
        Terrain::Water => [52, 96, 168, 255],
        Terrain::WoodFloor => [150, 110, 68, 255],
        Terrain::StoneBrick => [136, 132, 128, 255],
        Terrain::Snow => [228, 234, 244, 255],
        Terrain::CaveFloor => [78, 70, 88, 255],
        Terrain::Lava => [96, 24, 12, 255],
    }
}

fn draw_terrain_top(p: &mut Painter, t: Terrain, seed: u64) {
    let mut rng = Rng::new(seed);
    let base = terrain_base(t);
    p.fill_noise(base, &mut rng, 0.10);
    let n = TILE_PX as i32;
    match t {
        Terrain::Grass => {
            for _ in 0..26 {
                let x = rng.range(TILE_PX) as i32;
                let y = rng.range(TILE_PX) as i32;
                let c = if rng.chance(50) { shade(base, 1.3) } else { shade(base, 0.72) };
                p.px(x, y, c);
                if rng.chance(40) {
                    p.px(x, y - 1, shade(base, 1.4));
                }
            }
            for _ in 0..3 {
                let x = rng.range(TILE_PX) as i32;
                let y = rng.range(TILE_PX) as i32;
                p.px(x, y, [228, 208, 120, 255]); // tiny flowers
            }
        }
        Terrain::Dirt | Terrain::CaveFloor | Terrain::Sand => {
            for _ in 0..14 {
                let x = rng.range(TILE_PX) as i32;
                let y = rng.range(TILE_PX) as i32;
                p.disc(x, y, 1, shade(base, if rng.chance(50) { 0.8 } else { 1.2 }));
            }
        }
        Terrain::Stone => {
            for _ in 0..5 {
                let mut x = rng.range(TILE_PX) as i32;
                let mut y = rng.range(TILE_PX) as i32;
                for _ in 0..8 {
                    p.px(x, y, shade(base, 0.65));
                    x += rng.range(3) as i32 - 1;
                    y += 1;
                }
            }
        }
        Terrain::Water | Terrain::Lava => {
            let hi = if t == Terrain::Water { [120, 170, 224, 255] } else { [255, 150, 40, 255] };
            for i in 0..4 {
                let y = 4 + i * 8 + rng.range(3) as i32;
                for x in 0..n {
                    if (x + i * 5) % 9 < 5 {
                        p.px(x, y, hi);
                    }
                }
            }
            if t == Terrain::Lava {
                for _ in 0..6 {
                    let x = rng.range(TILE_PX) as i32;
                    let y = rng.range(TILE_PX) as i32;
                    p.disc(x, y, 1, [255, 220, 90, 255]);
                }
            }
        }
        Terrain::WoodFloor => {
            for row in 0..4 {
                let y = row * 8;
                p.rect(0, y, n, 1, shade(base, 0.55));
                let off = ((row * 13) % 32) as i32;
                p.rect(off, y, 1, 8, shade(base, 0.6));
                for x in 0..n {
                    if rng.chance(18) {
                        p.px(x, y + 2 + rng.range(5) as i32, shade(base, 0.85));
                    }
                }
            }
        }
        Terrain::StoneBrick => {
            for row in 0..4 {
                let y = row * 8;
                p.rect(0, y, n, 1, shade(base, 0.5));
                let off = if row % 2 == 0 { 8 } else { 0 };
                for bx in 0..3 {
                    p.rect(off + bx * 16, y, 1, 8, shade(base, 0.5));
                }
            }
            for _ in 0..10 {
                let x = rng.range(TILE_PX) as i32;
                let y = rng.range(TILE_PX) as i32;
                p.px(x, y, shade(base, 1.15));
            }
        }
        Terrain::Snow => {
            for _ in 0..10 {
                let x = rng.range(TILE_PX) as i32;
                let y = rng.range(TILE_PX) as i32;
                p.px(x, y, [255, 255, 255, 255]);
                p.px(x + 1, y, [200, 210, 230, 255]);
            }
        }
    }
    // Top edge light + bottom edge dark: fakes bevel between tiles.
    for x in 0..n {
        let idx_top = p.px_read(x, 0);
        p.px(x, 0, shade(idx_top, 1.12));
        let idx_bot = p.px_read(x, n - 1);
        p.px(x, n - 1, shade(idx_bot, 0.9));
    }
}

impl Painter<'_> {
    fn px_read(&self, x: i32, y: i32) -> Color {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return [0, 0, 0, 0];
        }
        let idx = (((self.oy + y as u32) * ATLAS_W + self.ox + x as u32) * 4) as usize;
        [self.atlas[idx], self.atlas[idx + 1], self.atlas[idx + 2], self.atlas[idx + 3]]
    }
}

fn draw_terrain_side(p: &mut Painter, t: Terrain, seed: u64) {
    let mut rng = Rng::new(seed);
    // Sides read as earth/cliff strata, tinted toward the top terrain.
    let top = terrain_base(t);
    let earth: Color = match t {
        Terrain::Stone | Terrain::StoneBrick => [96, 94, 104, 255],
        Terrain::CaveFloor => [58, 52, 68, 255],
        Terrain::Sand => [180, 152, 100, 255],
        Terrain::Snow => [148, 152, 172, 255],
        _ => [110, 82, 56, 255],
    };
    p.fill_noise(earth, &mut rng, 0.12);
    let n = TILE_PX as i32;
    // Thin cap of the top material.
    for x in 0..n {
        p.px(x, 0, shade(top, 0.9));
        if rng.chance(60) {
            p.px(x, 1, shade(top, 0.75));
        }
    }
    // Horizontal strata.
    for s in 1..4 {
        let y = s * 8 + rng.range(3) as i32;
        for x in 0..n {
            if rng.chance(80) {
                p.px(x, y, shade(earth, 0.72));
            }
        }
    }
    // Embedded stones.
    for _ in 0..6 {
        let x = rng.range(TILE_PX) as i32;
        let y = 4 + rng.range(TILE_PX - 4) as i32;
        p.disc(x, y, 1 + rng.range(2) as i32, shade(earth, if rng.chance(50) { 0.6 } else { 1.25 }));
    }
}

// ---------------------------------------------------------------------------
// Props (drawn bottom-anchored in a 32×48 cell)
// ---------------------------------------------------------------------------

fn draw_prop(p: &mut Painter, prop: Prop, seed: u64) {
    let mut rng = Rng::new(seed);
    let cx = (PROP_W / 2) as i32;
    let bottom = PROP_H as i32 - 1;
    match prop {
        Prop::None => {}
        Prop::Tree => {
            let trunk: Color = [102, 72, 46, 255];
            p.rect(cx - 2, bottom - 14, 4, 14, trunk);
            p.rect(cx - 1, bottom - 14, 1, 14, shade(trunk, 1.25));
            let leaf: Color = [64, 128, 60, 255];
            p.disc(cx, bottom - 24, 11, shade(leaf, 0.8));
            p.disc(cx - 5, bottom - 28, 8, leaf);
            p.disc(cx + 5, bottom - 27, 8, leaf);
            p.disc(cx, bottom - 33, 8, shade(leaf, 1.15));
            for _ in 0..30 {
                let a = rng.range(360) as f32 * 0.01745;
                let r = rng.range(10) as f32;
                let x = cx + (a.cos() * r) as i32;
                let y = bottom - 28 + (a.sin() * r * 0.8) as i32;
                p.px(x, y, shade(leaf, 1.3));
            }
        }
        Prop::Pine => {
            let trunk: Color = [92, 64, 40, 255];
            p.rect(cx - 1, bottom - 10, 3, 10, trunk);
            let leaf: Color = [40, 100, 70, 255];
            for layer in 0..4 {
                let w = 13 - layer * 3;
                let y = bottom - 12 - layer * 8;
                for row in 0..8 {
                    let rw = w - row;
                    if rw > 0 {
                        p.rect(cx - rw, y - row, rw * 2 + 1, 1, if row % 3 == 0 { shade(leaf, 1.2) } else { leaf });
                    }
                }
            }
        }
        Prop::Rock => {
            let c: Color = [128, 126, 134, 255];
            p.disc(cx, bottom - 6, 9, shade(c, 0.7));
            p.disc(cx - 2, bottom - 8, 7, c);
            p.disc(cx - 4, bottom - 10, 4, shade(c, 1.2));
            for _ in 0..8 {
                let x = cx - 8 + rng.range(16) as i32;
                let y = bottom - 12 + rng.range(10) as i32;
                p.px(x, y, shade(c, 0.85));
            }
        }
        Prop::Bush => {
            let leaf: Color = [70, 124, 58, 255];
            p.disc(cx - 4, bottom - 5, 5, shade(leaf, 0.8));
            p.disc(cx + 4, bottom - 5, 5, shade(leaf, 0.9));
            p.disc(cx, bottom - 8, 6, leaf);
            for _ in 0..12 {
                let x = cx - 8 + rng.range(16) as i32;
                let y = bottom - 12 + rng.range(10) as i32;
                p.px(x, y, shade(leaf, 1.3));
            }
        }
        Prop::Flowers => {
            for _ in 0..7 {
                let x = 4 + rng.range(PROP_W - 8) as i32;
                let y = bottom - 2 - rng.range(8) as i32;
                p.px(x, y + 1, [60, 110, 50, 255]);
                let c: Color = match rng.range(3) {
                    0 => [240, 120, 140, 255],
                    1 => [240, 220, 110, 255],
                    _ => [190, 140, 240, 255],
                };
                p.px(x, y, c);
                p.px(x + 1, y, shade(c, 0.85));
            }
        }
        Prop::Torch => {
            let pole: Color = [96, 70, 44, 255];
            p.rect(cx - 1, bottom - 18, 2, 18, pole);
            p.rect(cx - 2, bottom - 20, 4, 3, [70, 66, 72, 255]);
            // Bright flame → bloom picks it up.
            p.disc(cx, bottom - 23, 3, [255, 140, 30, 255]);
            p.disc(cx, bottom - 24, 2, [255, 220, 90, 255]);
            p.px(cx, bottom - 26, [255, 250, 180, 255]);
        }
        Prop::Signpost => {
            let wood: Color = [140, 104, 64, 255];
            p.rect(cx - 1, bottom - 14, 2, 14, shade(wood, 0.8));
            p.rect(cx - 8, bottom - 20, 16, 7, wood);
            p.rect(cx - 8, bottom - 20, 16, 1, shade(wood, 1.25));
            p.rect(cx - 6, bottom - 17, 12, 1, shade(wood, 0.6));
            p.rect(cx - 6, bottom - 15, 9, 1, shade(wood, 0.6));
        }
        Prop::Barrel => {
            let wood: Color = [134, 96, 58, 255];
            p.rect(cx - 6, bottom - 14, 12, 14, wood);
            p.rect(cx - 6, bottom - 14, 12, 1, shade(wood, 1.2));
            p.rect(cx - 6, bottom - 10, 12, 1, [90, 90, 100, 255]);
            p.rect(cx - 6, bottom - 4, 12, 1, [90, 90, 100, 255]);
            p.rect(cx - 2, bottom - 14, 1, 14, shade(wood, 1.15));
        }
        Prop::Crystal => {
            let c: Color = [110, 180, 255, 255];
            for (dx, h, w) in [(0i32, 22i32, 3i32), (-6, 14, 2), (6, 12, 2)] {
                for y in 0..h {
                    let ww = (w * (h - y) / h).max(1);
                    let col = if y > h - 5 { [220, 240, 255, 255] } else { shade(c, 0.8 + y as f32 * 0.02) };
                    p.rect(cx + dx - ww / 2, bottom - 1 - y, ww, 1, col);
                }
            }
        }
        Prop::Stump => {
            let wood: Color = [112, 82, 52, 255];
            p.rect(cx - 5, bottom - 8, 10, 8, shade(wood, 0.8));
            p.disc(cx, bottom - 8, 5, shade(wood, 1.2));
            p.disc(cx, bottom - 8, 3, wood);
            p.disc(cx, bottom - 8, 1, shade(wood, 0.7));
        }
        Prop::Cactus => {
            let c: Color = [92, 150, 80, 255];
            p.rect(cx - 2, bottom - 18, 4, 18, c);
            p.rect(cx - 8, bottom - 14, 3, 2, c);
            p.rect(cx - 8, bottom - 14, 2, 6, c);
            p.rect(cx + 5, bottom - 11, 3, 2, c);
            p.rect(cx + 6, bottom - 11, 2, 5, c);
            p.rect(cx - 1, bottom - 18, 1, 18, shade(c, 1.2));
        }
    }
}

// ---------------------------------------------------------------------------
// Characters — 16×24, 4 directions × 3 walk frames
// ---------------------------------------------------------------------------

struct CharStyle {
    skin: Color,
    hair: Color,
    tunic: Color,
    legs: Color,
    hat: Option<Color>,
    hood: bool,
    ghost: bool,
}

fn char_style(sheet: usize) -> CharStyle {
    match sheet {
        // 0 Warrior, 1 Mage, 2 Cleric, 3 Thief, 4 Elder, 5 Guard, 6 Ghost, 7 Kid
        0 => CharStyle { skin: [232, 190, 160, 255], hair: [104, 70, 40, 255], tunic: [170, 60, 54, 255], legs: [70, 60, 60, 255], hat: None, hood: false, ghost: false },
        1 => CharStyle { skin: [238, 200, 170, 255], hair: [220, 190, 90, 255], tunic: [64, 84, 180, 255], legs: [50, 54, 100, 255], hat: Some([54, 70, 160, 255]), hood: false, ghost: false },
        2 => CharStyle { skin: [230, 186, 156, 255], hair: [150, 100, 60, 255], tunic: [230, 228, 220, 255], legs: [180, 176, 168, 255], hat: None, hood: false, ghost: false },
        3 => CharStyle { skin: [224, 180, 150, 255], hair: [40, 40, 46, 255], tunic: [70, 130, 80, 255], legs: [46, 60, 46, 255], hat: None, hood: true, ghost: false },
        4 => CharStyle { skin: [226, 182, 152, 255], hair: [200, 200, 205, 255], tunic: [130, 90, 140, 255], legs: [90, 70, 100, 255], hat: None, hood: false, ghost: false },
        5 => CharStyle { skin: [228, 186, 156, 255], hair: [90, 70, 50, 255], tunic: [140, 144, 156, 255], legs: [90, 92, 102, 255], hat: Some([120, 124, 136, 255]), hood: false, ghost: false },
        6 => CharStyle { skin: [190, 220, 240, 200], hair: [210, 235, 250, 190], tunic: [160, 200, 235, 170], legs: [140, 180, 220, 150], hat: None, hood: true, ghost: true },
        _ => CharStyle { skin: [236, 196, 166, 255], hair: [180, 120, 60, 255], tunic: [220, 160, 70, 255], legs: [110, 80, 60, 255], hat: None, hood: false, ghost: false },
    }
}

fn draw_character(p: &mut Painter, sheet: usize, dir: u32, frame: u32) {
    let s = char_style(sheet);
    let cx = (CHAR_W / 2) as i32; // 8
    let bottom = CHAR_H as i32 - 1;

    // Walk cycle: frame 0/2 = step (legs apart), 1 = stand.
    let step = match frame {
        0 => -1i32,
        2 => 1,
        _ => 0,
    };
    let bob = if step != 0 { 1 } else { 0 };

    // Legs (skip for ghost — it trails off).
    if !s.ghost {
        let ly = bottom - 5;
        match dir {
            0 | 3 => {
                // down / up: legs side by side, alternate forward
                p.rect(cx - 3, ly + step.min(0), 2, 5 + step.abs().min(1), s.legs);
                p.rect(cx + 1, ly - step.max(0), 2, 5 + step.abs().min(1), s.legs);
            }
            _ => {
                // side: legs scissor
                p.rect(cx - 2 + step, ly, 2, 5, s.legs);
                p.rect(cx - 2 - step, ly + 1, 2, 4, shade(s.legs, 0.8));
            }
        }
    } else {
        // ghost tail
        for y in 0..5 {
            let w = 5 - y;
            p.rect(cx - w / 2 - 1, bottom - 5 + y, w, 1, shade(s.tunic, 0.9));
        }
    }

    // Body.
    let by = bottom - 12 - bob;
    p.rect(cx - 4, by, 8, 8, s.tunic);
    p.rect(cx - 4, by, 8, 1, shade(s.tunic, 1.2));
    p.rect(cx - 4, by + 7, 8, 1, shade(s.tunic, 0.75));
    // Arms.
    match dir {
        1 => p.rect(cx - 5 - step.min(0), by + 1, 2, 6, shade(s.tunic, 0.85)),
        2 => p.rect(cx + 3 + step.max(0), by + 1, 2, 6, shade(s.tunic, 0.85)),
        _ => {
            p.rect(cx - 5, by + 1 + step.max(0), 2, 5, shade(s.tunic, 0.85));
            p.rect(cx + 3, by + 1 - step.min(0), 2, 5, shade(s.tunic, 0.85));
        }
    }

    // Head.
    let hy = by - 8;
    p.rect(cx - 4, hy, 8, 8, s.skin);
    // Hair / hat / hood.
    if s.hood {
        p.rect(cx - 4, hy - 1, 8, 4, s.tunic);
        p.rect(cx - 5, hy + 1, 1, 4, s.tunic);
        p.rect(cx + 4, hy + 1, 1, 4, s.tunic);
    } else {
        p.rect(cx - 4, hy - 1, 8, 3, s.hair);
        if dir == 3 {
            p.rect(cx - 4, hy, 8, 6, s.hair); // back of head
        }
        if sheet == 4 {
            p.rect(cx - 5, hy + 1, 1, 5, s.hair); // elder's long hair
            p.rect(cx + 4, hy + 1, 1, 5, s.hair);
        }
    }
    if let Some(hat) = s.hat {
        p.rect(cx - 5, hy - 2, 10, 2, hat);
        p.rect(cx - 3, hy - 4, 6, 2, shade(hat, 1.15));
    }

    // Face (not when facing up).
    if dir != 3 {
        let eye: Color = if s.ghost { [40, 80, 140, 255] } else { [30, 26, 30, 255] };
        match dir {
            0 => {
                p.px(cx - 2, hy + 4, eye);
                p.px(cx + 1, hy + 4, eye);
            }
            1 => p.px(cx - 3, hy + 4, eye),
            _ => p.px(cx + 2, hy + 4, eye),
        }
    }
}

// ---------------------------------------------------------------------------
// Enemies — 48×48 battle sprites
// ---------------------------------------------------------------------------

fn draw_enemy(p: &mut Painter, which: usize, seed: u64) {
    let mut rng = Rng::new(seed);
    let n = ENEMY_PX as i32;
    let cx = n / 2;
    let bottom = n - 2;
    match which {
        0 => {
            // Slime.
            let c: Color = [96, 200, 110, 255];
            for y in 0..18 {
                let t = y as f32 / 18.0;
                let w = (16.0 + 6.0 * (1.0 - (1.0 - t) * (1.0 - t))) as i32;
                p.rect(cx - w / 2, bottom - 18 + y, w, 1, shade(c, 0.75 + t * 0.35));
            }
            p.disc(cx - 6, bottom - 20, 3, shade(c, 1.35));
            p.px(cx - 5, bottom - 12, [20, 40, 24, 255]);
            p.px(cx + 4, bottom - 12, [20, 40, 24, 255]);
            p.rect(cx - 2, bottom - 8, 5, 1, [20, 40, 24, 255]);
        }
        1 => {
            // Bat.
            let c: Color = [130, 90, 170, 255];
            p.disc(cx, bottom - 22, 7, c);
            for side in [-1i32, 1] {
                for i in 0..12 {
                    let x = cx + side * (7 + i);
                    let h = 10 - (i as f32 * 0.7) as i32 - if i % 4 == 0 { 2 } else { 0 };
                    p.rect(x, bottom - 26 + (i / 3), 1, h.max(2), shade(c, 0.8));
                }
            }
            p.px(cx - 3, bottom - 24, [255, 220, 60, 255]);
            p.px(cx + 3, bottom - 24, [255, 220, 60, 255]);
            for side in [-1i32, 1] {
                p.px(cx + side * 3, bottom - 29, shade(c, 0.9));
                p.px(cx + side * 4, bottom - 30, shade(c, 0.9));
            }
        }
        2 => {
            // Mud crawler.
            let c: Color = [150, 110, 70, 255];
            for seg in 0..3 {
                let sx = cx - 12 + seg * 12;
                p.disc(sx, bottom - 8, 7 - seg.min(1), shade(c, 0.85 + seg as f32 * 0.1));
            }
            p.disc(cx + 12, bottom - 10, 6, shade(c, 1.1));
            p.px(cx + 14, bottom - 12, [255, 60, 40, 255]);
            p.px(cx + 10, bottom - 12, [255, 60, 40, 255]);
            for i in 0..5 {
                p.rect(cx - 14 + i * 6, bottom - 2, 1, 2, shade(c, 0.6));
            }
        }
        _ => {
            // Stone golem.
            let c: Color = [138, 134, 146, 255];
            p.rect(cx - 10, bottom - 26, 20, 18, shade(c, 0.95)); // torso
            p.rect(cx - 7, bottom - 34, 14, 9, c); // head
            p.rect(cx - 15, bottom - 24, 5, 14, shade(c, 0.8)); // arms
            p.rect(cx + 10, bottom - 24, 5, 14, shade(c, 0.8));
            p.rect(cx - 8, bottom - 8, 6, 8, shade(c, 0.7)); // legs
            p.rect(cx + 2, bottom - 8, 6, 8, shade(c, 0.7));
            p.rect(cx - 4, bottom - 32, 3, 2, [120, 220, 255, 255]); // glowing eyes
            p.rect(cx + 2, bottom - 32, 3, 2, [120, 220, 255, 255]);
            for _ in 0..12 {
                let x = cx - 10 + rng.range(20) as i32;
                let y = bottom - 26 + rng.range(18) as i32;
                p.px(x, y, shade(c, if rng.chance(50) { 0.7 } else { 1.2 }));
            }
            p.rect(cx - 10, bottom - 20, 20, 1, shade(c, 0.65)); // crack
        }
    }
}
