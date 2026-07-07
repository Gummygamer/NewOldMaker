// NewOldMaker HD-2D shaders: scene (terrain + billboard sprites) and post chain.

// ---------------------------------------------------------------------------
// Shared scene uniforms
// ---------------------------------------------------------------------------

struct Light {
    pos_radius: vec4<f32>, // xyz = world pos, w = radius
    color: vec4<f32>,      // rgb = color * intensity
}

struct SceneUniforms {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    cam_right: vec4<f32>,   // xyz = world-space billboard right
    sun_dir: vec4<f32>,     // xyz = direction the sun shines toward
    sun_color: vec4<f32>,
    ambient: vec4<f32>,
    fog_color_density: vec4<f32>, // rgb, w = density
    misc: vec4<f32>,        // x = time, y = darkness, z,w unused
    light_count: vec4<u32>, // x = count
    lights: array<Light, 32>,
}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;
@group(0) @binding(1) var atlas_tex: texture_2d<f32>;
@group(0) @binding(2) var atlas_samp: sampler;

fn apply_lights(world_pos: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    let sun = max(dot(normal, -normalize(scene.sun_dir.xyz)), 0.0) * scene.sun_color.rgb;
    var light = scene.ambient.rgb + sun;
    let n = scene.light_count.x;
    for (var i = 0u; i < n; i = i + 1u) {
        let l = scene.lights[i];
        let to_l = l.pos_radius.xyz - world_pos;
        let d = length(to_l);
        let att = clamp(1.0 - d / max(l.pos_radius.w, 0.001), 0.0, 1.0);
        let ndl = max(dot(normal, to_l / max(d, 0.001)), 0.15); // wrap a little
        light += l.color.rgb * att * att * ndl;
    }
    return light;
}

fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let dist = length(world_pos - scene.cam_pos.xyz);
    let f = exp(-scene.fog_color_density.w * dist);
    return mix(scene.fog_color_density.rgb, color, clamp(f, 0.0, 1.0));
}

// ---------------------------------------------------------------------------
// Terrain
// ---------------------------------------------------------------------------

struct TerrainIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) flags: u32, // 1 = liquid (animated), 2 = emissive
}

struct TerrainOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) flags: u32,
}

@vertex
fn vs_terrain(in: TerrainIn) -> TerrainOut {
    var out: TerrainOut;
    var pos = in.pos;
    if ((in.flags & 1u) != 0u) {
        // Gentle liquid bob.
        pos.y += sin(scene.misc.x * 2.0 + pos.x * 1.7 + pos.z * 2.3) * 0.04;
    }
    out.clip = scene.view_proj * vec4<f32>(pos, 1.0);
    out.world_pos = pos;
    out.normal = in.normal;
    out.uv = in.uv;
    out.flags = in.flags;
    return out;
}

@fragment
fn fs_terrain(in: TerrainOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    if ((in.flags & 1u) != 0u) {
        // Scroll liquid texture inside its atlas cell (cell is 32px of a 1024x512 atlas).
        let cell = vec2<f32>(32.0 / 1024.0, 32.0 / 512.0);
        let t = scene.misc.x;
        let offs = vec2<f32>(fract(t * 0.06), fract(sin(t * 0.5) * 0.03 + 0.5) - 0.5);
        let base = floor(uv / cell) * cell;
        uv = base + fract((uv - base) / cell + offs) * cell;
    }
    var tex = textureSample(atlas_tex, atlas_samp, uv);
    var lit = tex.rgb * apply_lights(in.world_pos, in.normal);
    if ((in.flags & 2u) != 0u) {
        lit += tex.rgb * 1.6; // lava & friends glow through the dark
    }
    return vec4<f32>(apply_fog(lit, in.world_pos), 1.0);
}

// ---------------------------------------------------------------------------
// Billboard sprites (instanced)
// ---------------------------------------------------------------------------

struct SpriteIn {
    @builtin(vertex_index) vid: u32,
    @location(0) i_pos: vec3<f32>,
    @location(1) i_size: vec2<f32>,
    @location(2) i_uv0: vec2<f32>,
    @location(3) i_uv1: vec2<f32>,
    @location(4) i_tint: vec4<f32>,
    @location(5) i_flags: u32, // 1 = horizontal (shadow/cursor), 2 = unlit, 4 = emissive
}

struct SpriteOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) @interpolate(flat) flags: u32,
}

@vertex
fn vs_sprite(in: SpriteIn) -> SpriteOut {
    // Two-triangle quad from vertex index (0..5).
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, 0.0), vec2<f32>(0.5, 0.0), vec2<f32>(0.5, 1.0),
        vec2<f32>(-0.5, 0.0), vec2<f32>(0.5, 1.0), vec2<f32>(-0.5, 1.0),
    );
    let c = corners[in.vid];
    let right = normalize(scene.cam_right.xyz);
    var world: vec3<f32>;
    if ((in.i_flags & 1u) != 0u) {
        // Horizontal quad lying on the ground.
        let fwd = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), right));
        world = in.i_pos + right * c.x * in.i_size.x + fwd * (c.y - 0.5) * in.i_size.y;
    } else {
        // Upright billboard, rotates around Y toward camera.
        world = in.i_pos + right * c.x * in.i_size.x + vec3<f32>(0.0, 1.0, 0.0) * c.y * in.i_size.y;
    }
    var out: SpriteOut;
    out.clip = scene.view_proj * vec4<f32>(world, 1.0);
    out.world_pos = world;
    out.uv = vec2<f32>(
        mix(in.i_uv0.x, in.i_uv1.x, c.x + 0.5),
        mix(in.i_uv1.y, in.i_uv0.y, c.y), // v flipped: uv0 = top of sprite
    );
    out.tint = in.i_tint;
    out.flags = in.i_flags;
    return out;
}

@fragment
fn fs_sprite_cutout(in: SpriteOut) -> @location(0) vec4<f32> {
    let tex = textureSample(atlas_tex, atlas_samp, in.uv) * in.tint;
    if (tex.a < 0.5) {
        discard;
    }
    var lit = tex.rgb;
    if ((in.flags & 2u) == 0u) {
        lit *= apply_lights(in.world_pos, vec3<f32>(0.0, 1.0, 0.0));
    }
    if ((in.flags & 4u) != 0u) {
        lit += tex.rgb * 1.2;
    }
    return vec4<f32>(apply_fog(lit, in.world_pos), 1.0);
}

@fragment
fn fs_sprite_blend(in: SpriteOut) -> @location(0) vec4<f32> {
    let tex = textureSample(atlas_tex, atlas_samp, in.uv) * in.tint;
    var lit = tex.rgb;
    if ((in.flags & 2u) == 0u) {
        lit *= apply_lights(in.world_pos, vec3<f32>(0.0, 1.0, 0.0));
    }
    return vec4<f32>(lit, tex.a);
}

// ---------------------------------------------------------------------------
// Post chain
// ---------------------------------------------------------------------------

struct PostUniforms {
    // x = bloom_strength, y = bloom_threshold, z = focus_y (0..1), w = dof_strength
    a: vec4<f32>,
    // x = vignette, y = exposure, z = saturation, w = gamma (1.0 = srgb target)
    b: vec4<f32>,
    // x,y = texel size of source, z = blur radius scale, w = time
    c: vec4<f32>,
}

@group(0) @binding(0) var<uniform> post: PostUniforms;
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var src_samp: sampler;
@group(0) @binding(3) var aux_tex: texture_2d<f32>;

struct FsQuad {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vid: u32) -> FsQuad {
    // Single triangle covering the viewport.
    var out: FsQuad;
    let x = f32(i32(vid) / 2) * 4.0 - 1.0;
    let y = f32(i32(vid) % 2) * 4.0 - 1.0;
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@fragment
fn fs_downsample(in: FsQuad) -> @location(0) vec4<f32> {
    return textureSample(src_tex, src_samp, in.uv);
}

fn blur_1d(uv: vec2<f32>, dir: vec2<f32>) -> vec3<f32> {
    let texel = post.c.xy * post.c.z;
    var col = textureSample(src_tex, src_samp, uv).rgb * 0.227027;
    let offsets = array<f32, 4>(1.3846153846, 3.2307692308, 5.0769230769, 7.0);
    let weights = array<f32, 4>(0.3162162162, 0.0702702703, 0.0102, 0.0028);
    for (var i = 0; i < 4; i = i + 1) {
        let o = dir * texel * offsets[i];
        col += textureSample(src_tex, src_samp, uv + o).rgb * weights[i];
        col += textureSample(src_tex, src_samp, uv - o).rgb * weights[i];
    }
    return col;
}

@fragment
fn fs_blur_h(in: FsQuad) -> @location(0) vec4<f32> {
    return vec4<f32>(blur_1d(in.uv, vec2<f32>(1.0, 0.0)), 1.0);
}

@fragment
fn fs_blur_v(in: FsQuad) -> @location(0) vec4<f32> {
    return vec4<f32>(blur_1d(in.uv, vec2<f32>(0.0, 1.0)), 1.0);
}

// Final composite: src_tex = sharp HDR scene, aux_tex = blurred half-res scene.
@fragment
fn fs_composite(in: FsQuad) -> @location(0) vec4<f32> {
    let sharp = textureSample(src_tex, src_samp, in.uv).rgb;
    let soft = textureSample(aux_tex, src_samp, in.uv).rgb;

    // Tilt-shift: blur grows with distance from the horizontal focus band.
    let t = abs(in.uv.y - post.a.z);
    let dof = clamp(smoothstep(0.08, 0.42, t) * post.a.w, 0.0, 1.0);
    var col = mix(sharp, soft, dof);

    // Bloom from the blurred image's bright end.
    let bright = max(soft - vec3<f32>(post.a.y), vec3<f32>(0.0));
    col += bright * post.a.x;

    // Exposure + Reinhard tonemap.
    col *= post.b.y;
    col = col / (col + vec3<f32>(1.0));

    // Saturation.
    let grey = dot(col, vec3<f32>(0.299, 0.587, 0.114));
    col = mix(vec3<f32>(grey), col, post.b.z);

    // Vignette.
    let d = in.uv - vec2<f32>(0.5);
    col *= 1.0 - dot(d, d) * 2.0 * post.b.x;

    // Manual gamma when the target is not an sRGB format.
    if (post.b.w > 1.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}
