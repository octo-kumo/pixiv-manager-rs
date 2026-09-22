//! A GPU-blurred image tile for the "blur R-18" option, built on iced's
//! custom shader widget (`iced::widget::shader`).
//!
//! The thumbnail is drawn by a WGSL shader applying a 2D 5×5
//! linear-sampling Gaussian (25 taps). The blur radius is **animated**: the
//! widget's own tree state eases 0 ↔ [`MAX_RADIUS`] as the cursor enters or
//! leaves the tile, requesting redraw frames through the app's frame gate
//! until it settles. The blurred layer replaces the parallax image on tiles
//! it covers and shows the same centered cover crop (so the un-blurred end
//! state matches the tile at rest).
//!
//! GPU resources (textures + per-radius uniforms/bind groups) are cached in
//! the shared [`BlurRenderer`] pipeline, keyed by image handle id and a
//! quantized radius; `trim` clears them (iced calls it each frame end).

use iced::wgpu;
use iced::wgpu::util::{DeviceExt, TextureDataOrder};
use iced::widget::shader::{self, Action, Pipeline};
use iced::{mouse, time, window};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// The fully-blurred radius (screen px) used for R-18 tiles.
pub const MAX_RADIUS: f32 = 14.0;

/// Per-tick easing toward the target radius (a 16 ms tick → ~200 ms
/// transition, honoring the configured fps limit via the animation gate).
const ANIM_DECAY: f32 = 0.25;
/// The radius counts as settled within this many pixels.
const SETTLE_EPS: f32 = 0.25;

// ── Decoded-RGBA cache ─────────────────────────────────────────────────

/// Decoded RGBA8 pixels: `([width, height], bytes)`.
type Pixels = Arc<([u32; 2], Vec<u8>)>;

static PIXEL_CACHE: OnceLock<Mutex<HashMap<iced::advanced::image::Id, Pixels>>> = OnceLock::new();

/// Decode the handle's image into RGBA8 pixels, cached per handle id. The
/// decode runs on the UI thread when a blurred tile first appears (small
/// thumbnails — a millisecond or two); every later frame reuses the cache.
fn pixels_of(handle: &iced::widget::image::Handle) -> Option<Pixels> {
    let cache = PIXEL_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let id = handle.id();
    if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
        return Some(hit.clone());
    }
    let decoded: Pixels = match handle {
        iced::widget::image::Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } => Arc::new(([*width, *height], pixels.to_vec())),
        iced::widget::image::Handle::Bytes(_, bytes) => {
            let img = image::load_from_memory(bytes).ok()?;
            let rgba = img.to_rgba8();
            Arc::new(([rgba.width(), rgba.height()], rgba.into_raw()))
        }
        iced::widget::image::Handle::Path(_, path) => {
            let bytes = std::fs::read(path).ok()?;
            let img = image::load_from_memory(&bytes).ok()?;
            let rgba = img.to_rgba8();
            Arc::new((rgba.dimensions().into(), rgba.into_raw()))
        }
    };
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, decoded.clone());
    Some(decoded)
}

// ── Widget program ─────────────────────────────────────────────────────

/// The blur tile program: carries the (immutable) pixels and the target
/// radius; the animation itself lives in [`BlurState`].
pub struct BlurProgram {
    pixels: Pixels,
    /// The image handle id (keys the GPU texture cache).
    id: iced::advanced::image::Id,
    /// Blur target radius (screen px) when NOT hovered.
    max_radius: f32,
}

/// The widget's animation state (persists in the widget tree across
/// view rebuilds; `Default` starts un-initialized and fully blurred).
#[derive(Debug, Default, Clone, Copy)]
pub struct BlurState {
    hover: bool,
    /// Current animated radius (screen px).
    radius: f32,
    /// Whether the first redraw has seeded [`BlurState::radius`].
    inited: bool,
}

impl shader::Program<crate::app::message::Message> for BlurProgram {
    type State = BlurState;
    type Primitive = BlurPrimitive;

    fn update(
        &self,
        state: &mut BlurState,
        event: &iced::Event,
        bounds: iced::Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<shader::Action<crate::app::message::Message>> {
        match event {
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let hover = cursor.position().is_some_and(|p| bounds.contains(p));
                if hover != state.hover {
                    state.hover = hover;
                    return Some(Action::request_redraw());
                }
                None
            }
            iced::Event::Mouse(mouse::Event::CursorLeft) => {
                if state.hover {
                    state.hover = false;
                    return Some(Action::request_redraw());
                }
                None
            }
            iced::Event::Window(window::Event::RedrawRequested(_)) => {
                if !state.inited {
                    state.inited = true;
                    state.radius = if state.hover { 0.0 } else { self.max_radius };
                    return Some(Action::request_redraw());
                }
                let target = if state.hover { 0.0 } else { self.max_radius };
                if (state.radius - target).abs() > SETTLE_EPS {
                    state.radius += (target - state.radius) * ANIM_DECAY;
                    if (state.radius - target).abs() <= SETTLE_EPS {
                        state.radius = target;
                    }
                    // Animation frames respect the configured fps limit.
                    return Some(Action::request_redraw_at(
                        time::Instant::now() + crate::frame::animation_gate(),
                    ));
                }
                None
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        _cursor: mouse::Cursor,
        bounds: iced::Rectangle,
    ) -> Self::Primitive {
        // Before the first update tick the widget renders fully blurred —
        // it must never flash the sharp R-18 image.
        let radius = if state.inited {
            state.radius
        } else if state.hover {
            0.0
        } else {
            self.max_radius
        };
        // The crossfade tracks the radius: fully opaque when censored,
        // fully transparent when un-blurred.
        let fade = if self.max_radius > 0.0 {
            (radius / self.max_radius).clamp(0.0, 1.0)
        } else {
            0.0
        };
        BlurPrimitive {
            id: self.id,
            pixels: self.pixels.clone(),
            size: self.pixels.0,
            bounds_size: [bounds.width.max(1.0), bounds.height.max(1.0)],
            radius: radius.max(0.0),
            fade,
        }
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        mouse::Interaction::None
    }
}

// ── The GPU primitive + pipeline ───────────────────────────────────────

/// Draws a blurred (or sharp) cover-crop of the tile's thumbnail.
#[derive(Debug, Clone)]
pub struct BlurPrimitive {
    id: iced::advanced::image::Id,
    pixels: Pixels,
    size: [u32; 2],
    /// The tile bounds (logical px) — drives the cover crop.
    bounds_size: [f32; 2],
    /// Current blur radius (screen px); 0 = sharp.
    radius: f32,
    /// The layer's alpha [0,1]: 1 while censored, easing to 0 as the radius
    /// reaches 0 so the underlying (parallax) tile shows through — the
    /// un-blur becomes a crossfade instead of a hard swap.
    fade: f32,
}

/// The shared GPU state for every [`BlurPrimitive`].
pub struct BlurRenderer {
    bind_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    /// Uploaded textures, keyed by image handle id.
    textures: HashMap<iced::advanced::image::Id, wgpu::TextureView>,
    /// Uniform buffer + bind group per (texture id, quantized sigma).
    bindings: HashMap<u64, Binding>,
}

/// The per-(texture, blur-level) GPU binding.
#[derive(Debug)]
pub struct Binding {
    /// The uniform block backing `group`. The bind group holds its own
    /// reference (the buffer must outlive it); we keep it for clarity.
    #[allow(dead_code)]
    uniform: wgpu::Buffer,
    group: wgpu::BindGroup,
}

/// The binding-cache key: the image handle id (its debug form, hashed) XORed
/// with the quantized sigma in the high bits. Radius quantization (½ px
/// steps) keeps an animated transition within a small, bounded set of
/// buffers.
fn binding_key(id: iced::advanced::image::Id, sigma: u32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&id, &mut hasher);
    let hashed = std::hash::Hasher::finish(&hasher);
    (hashed & 0x0000_0000_FFFF_FFFF) | ((sigma as u64) << 32)
}

/// The classic linear-sampling 9-tap Gaussian positions (in sigma units);
/// applied on both axes with the bilinear trick → 25 samples.
const TAPS: [f32; 5] = [0.0, 1.3846, -1.3846, 3.2308, -3.2308];
const TAP_WEIGHTS: [f32; 5] = [0.2270, 0.3162, 0.3162, 0.0703, 0.0703];

/// The uniform block (144 bytes): 5 tap offsets, 5 tap weights, then
/// `(1/w, 1/h, region_w, region_h)` and the blur-layer fade alpha.
const UNIFORM_SIZE: u64 = 144;

const WGSL: &str = r#"
struct Params {
    offsets: array<vec4<f32>, 5>,  // .x = texel offset (used on both axes)
    weights: array<vec4<f32>, 2>,  // w0..w3, then w4
    texel: vec4<f32>,              // (1/w, 1/h, region_w, region_h)
    fade: vec4<f32>,               // x: layer alpha (the un-blur crossfade)
}

@group(0) @binding(0) var t_sampler: sampler;
@group(0) @binding(1) var t_texture: texture_2d<f32>;
@group(0) @binding(2) var<uniform> params: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    var quad = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0),
        vec2( 1.0, -1.0),
        vec2(-1.0,  1.0),
        vec2(-1.0,  1.0),
        vec2( 1.0, -1.0),
        vec2( 1.0,  1.0),
    );
    let xy = quad[i];
    // uv (0,0) is the image's top-left; clip-space Y is up.
    return VsOut(
        vec4<f32>(xy, 0.0, 1.0),
        vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5),
    );
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    // Centered cover crop: map the tile's [0,1] uv into the sub-region of
    // the source that fills the tile without distortion.
    let uv = vec2<f32>(0.5) + (in.uv - 0.5) * params.texel.zw;

    var sum = vec4<f32>(0.0);
    for (var j = 0u; j < 5u; j++) {
        for (var i = 0u; i < 5u; i++) {
            let w = params.weights[i / 4u][i % 4u] * params.weights[j / 4u][j % 4u];
            let off = vec2<f32>(
                params.offsets[i].x * params.texel.x,
                params.offsets[j].x * params.texel.y,
            );
            sum += w * textureSample(t_texture, t_sampler, uv + off);
        }
    }
    return vec4<f32>(sum.rgb, sum.a * params.fade.x);
}
"#;

impl Pipeline for BlurRenderer {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let _ = queue;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pixiv-manager-blur"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pixiv-manager-blur-bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pixiv-manager-blur-layout"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pixiv-manager-blur-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pixiv-manager-blur-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        Self {
            bind_layout,
            pipeline,
            sampler,
            textures: HashMap::new(),
            bindings: HashMap::new(),
        }
    }

    fn trim(&mut self) {
        self.textures.clear();
        self.bindings.clear();
    }
}

impl shader::Primitive for BlurPrimitive {
    type Pipeline = BlurRenderer;

    fn prepare(
        &self,
        renderer: &mut BlurRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _bounds: &iced::Rectangle,
        _viewport: &shader::Viewport,
    ) {
        let (iw, ih) = (self.size[0].max(1), self.size[1].max(1));

        // Upload the texture once per image (the pipeline cache survives
        // frames; `trim` evicts it).
        let view = match renderer.textures.entry(self.id) {
            std::collections::hash_map::Entry::Occupied(occupied) => occupied.into_mut(),
            std::collections::hash_map::Entry::Vacant(slot) => {
                let texture = device.create_texture_with_data(
                    queue,
                    &wgpu::TextureDescriptor {
                        label: Some("pixiv-manager-blur-texture"),
                        size: wgpu::Extent3d {
                            width: iw,
                            height: ih,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    TextureDataOrder::LayerMajor,
                    &self.pixels.1,
                );
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                slot.insert(view)
            }
        };

        // The per-sigma uniform + bind group (radius quantized to ½ px so
        // an animated transition reuses a small, bounded set of buffers).
        let key = binding_key(self.id, sigma_key(self.radius));
        if !renderer.bindings.contains_key(&key) {
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pixiv-manager-blur-uniform"),
                size: UNIFORM_SIZE,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&uniform, 0, &self.uniform());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pixiv-manager-blur-bind-group"),
                layout: &renderer.bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Sampler(&renderer.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            });
            renderer.bindings.insert(
                key,
                Binding {
                    uniform,
                    group: bind_group,
                },
            );
        }
    }

    fn draw(&self, renderer: &BlurRenderer, pass: &mut wgpu::RenderPass<'_>) -> bool {
        let key = binding_key(self.id, sigma_key(self.radius));
        let Some(binding) = renderer.bindings.get(&key) else {
            return false;
        };
        pass.set_pipeline(&renderer.pipeline);
        pass.set_bind_group(0, &binding.group, &[]);
        pass.draw(0..6, 0..1);
        true
    }
}

fn sigma_key(radius: f32) -> u32 {
    (radius.max(0.0) * 2.0).round() as u32
}

impl BlurPrimitive {
    /// The centered cover-crop uv region `(region_w, region_h)` of the
    /// source image that fills the tile — the same crop the parallax tile
    /// uses at rest.
    fn cover(&self) -> [f32; 2] {
        let (iw, ih) = (self.size[0] as f32, self.size[1] as f32);
        if iw <= 0.0 || ih <= 0.0 {
            return [1.0, 1.0];
        }
        let scale = (self.bounds_size[0] / iw).max(self.bounds_size[1] / ih);
        [
            self.bounds_size[0] / (iw * scale),
            self.bounds_size[1] / (ih * scale),
        ]
    }

    /// The blur sigma in SOURCE texels that reads as `radius` screen px.
    fn sigma_texels(&self) -> f32 {
        if self.radius <= 0.0 {
            return 0.0;
        }
        let (iw, ih) = (self.size[0] as f32, self.size[1] as f32);
        let scale = (self.bounds_size[0] / iw).max(self.bounds_size[1] / ih);
        // One source texel spans `scale` screen px along the displayed axis
        // (the cover scale is uniform; the tile may crop the other axis).
        let texel_screen_px = scale.max(0.25);
        (self.radius / texel_screen_px).max(0.0)
    }

    /// The 128-byte uniform block for the current radius/crop.
    ///
    /// WGSL uniform layout (16-byte strides!): `offsets[5]` occupies bytes
    /// 0–80 — each array element is a vec4, so only the `.x` lane of each is
    /// used; `weights[2]` is bytes 80–112; `texel` is bytes 112–128. Writing
    /// the taps as packed f32s instead left the GPU reading zero weights —
    /// the shader summed to transparent and the tile rendered as a grey
    /// panel (the "grey rectangle, no blur" bug).
    fn uniform(&self) -> [u8; UNIFORM_SIZE as usize] {
        let mut data = [0.0f32; UNIFORM_SIZE as usize / 4];
        let sigma = self.sigma_texels();
        for (i, tap) in TAPS.iter().enumerate() {
            data[i * 4] = tap * sigma;
        }
        for (i, w) in TAP_WEIGHTS.iter().enumerate() {
            data[20 + i] = *w;
        }
        let texel = [1.0 / self.size[0] as f32, 1.0 / self.size[1] as f32];
        let region = self.cover();
        data[28] = texel[0];
        data[29] = texel[1];
        data[30] = region[0];
        data[31] = region[1];
        data[32] = self.fade.clamp(0.0, 1.0);
        let mut bytes = [0u8; UNIFORM_SIZE as usize];
        for (i, v) in data.iter().enumerate() {
            bytes[i * 4..(i + 1) * 4].copy_from_slice(&v.to_ne_bytes());
        }
        bytes
    }
}

/// A tile-sized blurred image (the R-18 censoring layer, animating to sharp
/// while hovered).
pub fn blur_image(
    handle: &iced::widget::image::Handle,
    width: f32,
    height: f32,
) -> Option<iced::Element<'static, crate::app::message::Message>> {
    let pixels = pixels_of(handle)?;
    Some(
        iced::widget::shader::Shader::new(BlurProgram {
            id: handle.id(),
            pixels,
            max_radius: MAX_RADIUS,
        })
        .width(width)
        .height(height)
        .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn primitive(radius: f32) -> BlurPrimitive {
        primitive_with_fade(radius, radius / MAX_RADIUS)
    }

    fn primitive_with_fade(radius: f32, fade: f32) -> BlurPrimitive {
        // A stable key: `pixels_of` caches by handle id, but the test builds
        // the primitive directly — any id works.
        let id = iced::widget::image::Handle::from_bytes(vec![0u8]).id();
        BlurPrimitive {
            id,
            pixels: Arc::new(([64, 64], vec![0u8; 64 * 64 * 4])),
            size: [64, 64],
            bounds_size: [200.0, 200.0],
            radius,
            fade,
        }
    }

    /// Regression for the "grey rectangle, no blur" bug: the WGSL uniform
    /// struct lays `array<vec4<f32>, 5>` out at byte 0 with a 16-byte stride
    /// per element. Packing the taps and weights back-to-back (5 floats for
    /// offsets, 5 for weights) made the GPU read zero weights — every blur
    /// sum was transparent and the tile fell back to the panel background.
    #[test]
    fn uniform_matches_the_wgsl_layout() {
        let bytes = primitive_with_fade(MAX_RADIUS, 1.0).uniform();
        let f = |i: usize| f32::from_ne_bytes(bytes[i * 4..(i + 1) * 4].try_into().unwrap());

        // offsets[i] lives in vec4 element i (float index i * 4).
        let sigma = primitive_with_fade(MAX_RADIUS, 1.0).sigma_texels();
        for (i, tap) in TAPS.iter().enumerate() {
            assert_eq!(f(i * 4), tap * sigma, "offset tap {i} at float {}", i * 4);
        }
        // weights[2] occupies floats 20..28 (bytes 80..112).
        for (i, w) in TAP_WEIGHTS.iter().enumerate() {
            assert_eq!(f(20 + i), *w, "weight {i} at float {}", 20 + i);
        }
        // texel/region occupy floats 28..32 (bytes 112..128); fade is at
        // float 32 (bytes 128..132).
        assert!((f(28) - 1.0 / 64.0).abs() < 1e-6, "texel.x");
        assert!((f(29) - 1.0 / 64.0).abs() < 1e-6, "texel.y");
        // A square source in a square tile: the cover region is the whole
        // image.
        assert_eq!((f(30), f(31)), (1.0, 1.0), "cover region");
        assert!((f(32) - 1.0).abs() < 1e-6, "fade at full censure");
        // The unused y/z/w lanes of each vec4 element are padding (zeros):
        // floats 1–3, 5–7, 9–11, 13–15, 17–19 (and weights[1].y–w).
        for i in 0..36 {
            if matches!(
                i,
                0 | 4 | 8 | 12 | 16 | 20 | 21 | 22 | 23 | 24 | 28 | 29 | 30 | 31 | 32
            ) {
                continue; // taps, weights, texel/region, fade
            }
            assert_eq!(f(i), 0.0, "float {i} is padding");
        }
    }

    #[test]
    fn sigma_scales_with_the_requested_radius() {
        // Radius 0 is sharp (all tap offsets collapse to the center);
        // a bigger radius widens the kernel; weights never depend on it.
        let sharp = primitive(0.0);
        let blurred = primitive(12.0);
        assert_eq!(sharp.sigma_texels(), 0.0);
        assert!(blurred.sigma_texels() > 0.0);

        let f = |p: &BlurPrimitive, i: usize| {
            f32::from_ne_bytes(p.uniform()[i * 4..(i + 1) * 4].try_into().unwrap())
        };
        for i in 0..5 {
            assert_eq!(tap_offset(&sharp, i), 0.0, "sharp offset tap {i}");
        }
        // The fade is passed through to the uniform verbatim: radius 0 →
        // transparent (the parallax shows), full blur → opaque.
        let opaque = primitive_with_fade(MAX_RADIUS, 1.0);
        assert_eq!(f(&opaque, 32), 1.0);
        let transparent = primitive_with_fade(0.0, 0.0);
        assert_eq!(f(&transparent, 32), 0.0);
    }

    /// The x-lane of tap `i` in the uniform block.
    fn tap_offset(p: &BlurPrimitive, tap: usize) -> f32 {
        let bytes = p.uniform();
        f32::from_ne_bytes(bytes[tap * 4..(tap + 1) * 4].try_into().unwrap())
    }
}
