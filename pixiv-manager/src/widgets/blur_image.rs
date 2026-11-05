//! A censored image tile for the "blur R-18" option, built on iced's custom
//! shader widget (`iced::widget::shader`).
//!
//! The blur itself is a real Gaussian, not a sparse in-shader approximation
//! (which produced echoes and ringing at large radii): each thumbnail is
//! blurred once on the CPU into a small, smooth copy, and the shader only
//! crossfades between the sharp and the pre-blurred texture. Two texture
//! samples per pixel, and the blur is as smooth as the halo radius asks for.
//!
//! The censor amount is **animated**: the widget's own tree state eases
//! 0 ↔ [`MAX_RADIUS`] as the cursor enters or leaves the tile, requesting
//! redraw frames through the app's frame gate until it settles. The censored
//! layer replaces the parallax image on the tiles it covers and shows the
//! same centered cover crop (so the un-blurred end state matches the tile at
//! rest).
//!
//! GPU resources (two textures + per-fade uniforms/bind groups) are cached in
//! the shared [`BlurRenderer`] pipeline, keyed by image handle id and a
//! quantized fade; `trim` clears them (iced calls it each frame end).

use iced::wgpu;
use iced::wgpu::util::{DeviceExt, TextureDataOrder};
use iced::widget::shader::{self, Action, Pipeline};
use iced::{mouse, time, window};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// The fully-blurred radius (screen px) used for R-18 tiles.
pub const MAX_RADIUS: f32 = 14.0;

/// Maximum dimension of the pre-blurred censor texture. A blurred image is
/// low-frequency, so it can be generated at this size and upscaled by the
/// GPU: a 540 px thumbnail needs only a ~160 px blurred copy.
const BLUR_TEXTURE_MAX: u32 = 160;

/// The visible full-strength Gaussian sigma as a fraction of the tile
/// (1/24 of the tile's larger side): strong enough to obscure an illustration
/// while keeping the composition recognisable.
const BLUR_SIGMA_FRACTION: f32 = 1.0 / 24.0;

/// Per-tick easing toward the target radius (a 16 ms tick → ~200 ms
/// transition, honoring the configured fps limit via the animation gate).
const ANIM_DECAY: f32 = 0.25;
/// The radius counts as settled within this many pixels.
const SETTLE_EPS: f32 = 0.25;

// ── Decoded-RGBA cache ─────────────────────────────────────────────────

/// One decoded thumbnail plus its pre-blurred censoring copy.
#[derive(Debug)]
struct Decoded {
    /// `[width, height]` of the sharp texture.
    size: [u32; 2],
    /// RGBA8 pixels of the sharp thumbnail.
    sharp: Vec<u8>,
    /// The pre-blurred censor copy: `([width, height], RGBA8)`.
    blurred: ([u32; 2], Vec<u8>),
}

type Pixels = Arc<Decoded>;

static PIXEL_CACHE: OnceLock<Mutex<HashMap<iced::advanced::image::Id, Pixels>>> = OnceLock::new();

/// Decode the handle's image into RGBA8 pixels and pre-blur it, cached per
/// handle id. The work runs on the UI thread when a censored tile first
/// appears (small thumbnails — a millisecond or two); every later frame
/// reuses the cache.
fn pixels_of(handle: &iced::widget::image::Handle) -> Option<Pixels> {
    let cache = PIXEL_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let id = handle.id();
    if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
        return Some(hit.clone());
    }
    let (size, sharp): ([u32; 2], Vec<u8>) = match handle {
        iced::widget::image::Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } => ([*width, *height], pixels.to_vec()),
        iced::widget::image::Handle::Bytes(_, bytes) => {
            let img = image::load_from_memory(bytes).ok()?;
            let rgba = img.to_rgba8();
            ([rgba.width(), rgba.height()], rgba.into_raw())
        }
        iced::widget::image::Handle::Path(_, path) => {
            let bytes = std::fs::read(path).ok()?;
            let img = image::load_from_memory(&bytes).ok()?;
            let rgba = img.to_rgba8();
            ([rgba.width(), rgba.height()], rgba.into_raw())
        }
    };
    let blurred = blurred_of(size, &sharp)?;
    let decoded = Arc::new(Decoded {
        size,
        sharp,
        blurred,
    });
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, decoded.clone());
    Some(decoded)
}

/// Pre-blur a decoded thumbnail into a small, smooth Gaussian RGBA copy used
/// as the censoring layer.
///
/// This implements the standard real-time large-radius blur strategy (see
/// e.g. Intel's "Investigation of fast real-time GPU-based image blur
/// algorithms", and mip/dual-filtering blur): downsample first, convolve the
/// low-frequency result with a separable Gaussian kernel, then let the GPU's
/// bilinear filtering upsample it. The blur is baked once per thumbnail and
/// cached, so the per-frame cost is just two texture samples in the shader;
/// running the kernel per frame would be the expensive way.
fn blurred_of(size: [u32; 2], sharp: &[u8]) -> Option<([u32; 2], Vec<u8>)> {
    let (w, h) = (size[0], size[1]);
    if w == 0 || h == 0 {
        return None;
    }
    let rgba = image::RgbaImage::from_raw(w, h, sharp.to_vec())?;
    let max = w.max(h);
    let scale = (BLUR_TEXTURE_MAX as f32 / max as f32).min(1.0);
    let bw = ((w as f32 * scale).round() as u32).max(1);
    let bh = ((h as f32 * scale).round() as u32).max(1);
    let small = if (bw, bh) == (w, h) {
        rgba
    } else {
        image::imageops::resize(&rgba, bw, bh, image::imageops::FilterType::Triangle)
    };
    // Sigma relative to the texture so the on-screen blur scales with the
    // tile (a 200 px tile gets sigma ≈ 8 px, a 400 px tile ≈ 17 px).
    let sigma = (bw.max(bh) as f32 * BLUR_SIGMA_FRACTION).max(0.5);
    // Separable Gaussian convolution; the source is downscaled first, so
    // even a wide kernel stays trivially cheap (160×160 tops).
    let blurred = image::imageops::blur(&small, sigma);
    Some(([blurred.width(), blurred.height()], blurred.into_raw()))
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
            size: self.pixels.size,
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
    /// Uploaded textures (sharp + pre-blurred), keyed by image handle id.
    textures: HashMap<iced::advanced::image::Id, Views>,
    /// Uniform buffer + bind group per (texture id, quantized fade).
    bindings: HashMap<u64, Binding>,
    /// Textures/bindings used since the last `trim`, so the per-frame trim
    /// only evicts data that is actually gone from the screen instead of
    /// re-uploading every censored tile every frame.
    used_textures: std::collections::HashSet<iced::advanced::image::Id>,
    used_bindings: std::collections::HashSet<u64>,
}

/// The two texture views a censored tile samples: the sharp thumbnail and its
/// pre-blurred censor copy.
#[derive(Debug)]
struct Views {
    sharp: wgpu::TextureView,
    blurred: wgpu::TextureView,
}

/// The per-(texture, fade-level) GPU binding.
#[derive(Debug)]
pub struct Binding {
    /// The uniform block backing `group`. The bind group holds its own
    /// reference (the buffer must outlive it); we keep it for clarity.
    #[allow(dead_code)]
    uniform: wgpu::Buffer,
    group: wgpu::BindGroup,
}

/// The binding-cache key: the image handle id (its debug form, hashed) XORed
/// with the quantized censor amount in the high bits. Quantization (½ px
/// steps) keeps an animated transition within a small, bounded set of
/// buffers.
fn binding_key(id: iced::advanced::image::Id, level: u32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&id, &mut hasher);
    let hashed = std::hash::Hasher::finish(&hasher);
    (hashed & 0x0000_0000_FFFF_FFFF) | ((level as u64) << 32)
}

/// The uniform block (32 bytes): the cover region `(region_w, region_h)` and
/// the censor-layer fade alpha.
const UNIFORM_SIZE: u64 = 32;

const WGSL: &str = r#"
struct Params {
    region: vec4<f32>, // .z/.w = centered cover region of the source
    fade: vec4<f32>,   // .x = censor alpha (1 = blurred, 0 = sharp)
}

@group(0) @binding(0) var t_sampler: sampler;
@group(0) @binding(1) var t_sharp: texture_2d<f32>;
@group(0) @binding(2) var t_blurred: texture_2d<f32>;
@group(0) @binding(3) var<uniform> params: Params;

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
    let uv = vec2<f32>(0.5) + (in.uv - vec2(0.5)) * params.region.zw;
    let sharp = textureSample(t_sharp, t_sampler, uv);
    let blurred = textureSample(t_blurred, t_sampler, uv);
    return vec4<f32>(mix(sharp.rgb, blurred.rgb, params.fade.x), 1.0);
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
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
            used_textures: std::collections::HashSet::new(),
            used_bindings: std::collections::HashSet::new(),
        }
    }

    fn trim(&mut self) {
        // iced calls this at the end of every frame. Evict only what was not
        // drawn during that frame; hard-clearing everything forced a texture
        // upload for every censored tile on every frame.
        self.textures
            .retain(|id, _| self.used_textures.contains(id));
        self.bindings
            .retain(|key, _| self.used_bindings.contains(key));
        self.used_textures.clear();
        self.used_bindings.clear();
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

        // Upload both textures once per image (the pipeline cache survives
        // frames; `trim` evicts it). The formats match iced's image atlas for
        // this build, so the censored layer has exactly the same brightness
        // as the normal tile.
        renderer.used_textures.insert(self.id);
        let views = match renderer.textures.entry(self.id) {
            std::collections::hash_map::Entry::Occupied(occupied) => occupied.into_mut(),
            std::collections::hash_map::Entry::Vacant(slot) => {
                let create = |label: &'static str, size: [u32; 2], data: &[u8]| {
                    device.create_texture_with_data(
                        queue,
                        &wgpu::TextureDescriptor {
                            label: Some(label),
                            size: wgpu::Extent3d {
                                width: size[0].max(1),
                                height: size[1].max(1),
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: texture_format(),
                            usage: wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        },
                        TextureDataOrder::LayerMajor,
                        data,
                    )
                };
                let sharp_texture = create(
                    "pixiv-manager-blur-sharp",
                    self.pixels.size,
                    &self.pixels.sharp,
                );
                let blurred_texture = create(
                    "pixiv-manager-blur-blurred",
                    self.pixels.blurred.0,
                    &self.pixels.blurred.1,
                );
                let views = Views {
                    sharp: sharp_texture.create_view(&wgpu::TextureViewDescriptor::default()),
                    blurred: blurred_texture.create_view(&wgpu::TextureViewDescriptor::default()),
                };
                slot.insert(views)
            }
        };
        let _ = (iw, ih);

        // The per-fade uniform + bind group (the censor amount is quantized
        // so an animated transition reuses a small, bounded set of buffers).
        let key = binding_key(self.id, fade_key(self.radius));
        renderer.used_bindings.insert(key);
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
                        resource: wgpu::BindingResource::TextureView(&views.sharp),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&views.blurred),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
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
        let key = binding_key(self.id, fade_key(self.radius));
        let Some(binding) = renderer.bindings.get(&key) else {
            return false;
        };
        pass.set_pipeline(&renderer.pipeline);
        pass.set_bind_group(0, &binding.group, &[]);
        pass.draw(0..6, 0..1);
        true
    }
}

fn fade_key(radius: f32) -> u32 {
    (radius.max(0.0) * 2.0).round() as u32
}

/// The texture format that matches iced's image atlas in this build.
///
/// With iced's default `web-colors` feature gamma correction is disabled and
/// both the atlas and the window surface are non-sRGB. An `Rgba8UnormSrgb`
/// texture is decoded to linear space when sampled but then written to a
/// non-sRGB target unchanged, which made the censored tile render visibly
/// darker than the sharp image next to it (and brighten as it un-blurred).
/// Follow the renderer's gamma setting instead of hard-coding one format.
fn texture_format() -> wgpu::TextureFormat {
    if iced::advanced::graphics::color::GAMMA_CORRECTION {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    }
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

    /// The 32-byte uniform block for the current cover crop + fade.
    ///
    /// WGSL layout (16-byte strides): `region: vec4` occupies bytes 0–16
    /// (only `.z`/`.w` are used — the cover region), then `fade: vec4` at
    /// bytes 16–32 (only `.x` — the censor alpha).
    fn uniform(&self) -> [u8; UNIFORM_SIZE as usize] {
        let mut data = [0.0f32; UNIFORM_SIZE as usize / 4];
        let region = self.cover();
        data[2] = region[0];
        data[3] = region[1];
        data[4] = self.fade.clamp(0.0, 1.0);
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
            pixels: Arc::new(Decoded {
                size: [64, 64],
                sharp: vec![0u8; 64 * 64 * 4],
                blurred: ([8, 8], vec![0u8; 8 * 8 * 4]),
            }),
            size: [64, 64],
            bounds_size: [200.0, 200.0],
            radius,
            fade,
        }
    }

    fn float_at(p: &BlurPrimitive, i: usize) -> f32 {
        let bytes = p.uniform();
        f32::from_ne_bytes(bytes[i * 4..(i + 1) * 4].try_into().unwrap())
    }

    #[test]
    fn uniform_matches_the_wgsl_layout() {
        let p = primitive_with_fade(MAX_RADIUS, 1.0);
        // A square source in a square tile: the cover region is the whole
        // image. `region` is the first vec4 (floats 0..4), using .z/.w.
        assert_eq!((float_at(&p, 2), float_at(&p, 3)), (1.0, 1.0), "region");
        assert_eq!(float_at(&p, 0), 0.0, "region.x is padding");
        assert_eq!(float_at(&p, 1), 0.0, "region.y is padding");
        // The censor alpha is `fade.x` (float 4 of 8).
        assert_eq!(float_at(&p, 4), 1.0, "fade at full censure");
        for i in [5, 6, 7] {
            assert_eq!(float_at(&p, i), 0.0, "fade padding at float {i}");
        }
        assert_eq!(UNIFORM_SIZE, 32, "two vec4s");
    }

    #[test]
    fn texture_format_matches_the_renderer_gamma_mode() {
        // Regression: the sRGB format in a non-sRGB pipeline darkened every
        // censored tile. The format must follow iced's gamma correction.
        assert_eq!(
            texture_format().is_srgb(),
            iced::advanced::graphics::color::GAMMA_CORRECTION
        );
    }

    #[test]
    fn blurred_copy_is_small_and_smooth() {
        // 320×320, left half black, right half white.
        let (w, h) = (320u32, 320u32);
        let mut sharp = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let v = if x < w / 2 { 0 } else { 255 };
                sharp[i..i + 3].copy_from_slice(&[v, v, v]);
                sharp[i + 3] = 255;
            }
        }
        let ([bw, bh], blurred) = blurred_of([w, h], &sharp).expect("blur");
        assert_eq!(
            [bw, bh],
            [BLUR_TEXTURE_MAX, BLUR_TEXTURE_MAX],
            "the censor copy is downscaled to the blur-texture cap"
        );
        // A real Gaussian has a gradual edge, not a hard step. The original
        // boundary (x = w/2) sits at the middle of the downscaled copy.
        let row = bh as usize / 2;
        let border = bw as usize / 2;
        let px = |x: usize| blurred[(row * bw as usize + x) * 4] as i32;
        assert!(px(border - 10) < 100, "dark side is blurred");
        assert!(px(border + 10) > 155, "light side is blurred");
        let mid = px(border);
        assert!(
            (20..235).contains(&mid),
            "edge is gradual, not stepped: {mid}"
        );
        let soft = (border - 8..border + 9)
            .filter(|x| (5..250).contains(&px(*x)))
            .count();
        assert!(soft >= 5, "only {soft} soft pixels across the edge");
    }

    #[test]
    fn cover_crop_centers_the_image() {
        let p = primitive(MAX_RADIUS);
        assert_eq!(p.cover(), [1.0, 1.0]);
        let mut wide = primitive(MAX_RADIUS);
        wide.size = [400, 200];
        assert_eq!(wide.cover(), [0.5, 1.0]);
    }

    #[test]
    fn fade_is_written_to_the_uniform() {
        assert_eq!(float_at(&primitive_with_fade(MAX_RADIUS, 1.0), 4), 1.0);
        assert_eq!(float_at(&primitive_with_fade(0.0, 0.0), 4), 0.0);
    }
}
