// GPU and snapshot setup failures must fail this test.
#![allow(clippy::unwrap_used)]

use super::snap_with;
use rune::color::{hex, srgb_decode};
use rune::render::Gpu;
use rune::{Cx, El, effect, el};
use rune_fx::*;
use std::sync::Arc;

const SIZE: u32 = 96;
const AMOUNT: f32 = 0.06;

#[derive(Debug)]
struct Balance {
    up: f64,
    down: f64,
}

impl Balance {
    fn visible(&self) -> bool {
        self.up > 0.005 && self.down > 0.005
    }

    fn symmetric(&self) -> bool {
        (self.up - self.down).abs() <= 0.1 * (self.up + self.down) / 2.0
    }

    fn imbalance(&self) -> f64 {
        2.0 * (self.up - self.down).abs() / (self.up + self.down)
    }
}

fn balance(pixels: impl Iterator<Item = (f64, f64)>) -> Balance {
    let (mut up, mut down, mut count) = (0.0, 0.0, 0);
    for (value, base) in pixels {
        if value > base {
            up += (value - base) / (1.0 - base);
        } else {
            down += (base - value) / base;
        }
        count += 1;
    }
    assert!(count > 0);
    Balance {
        up: up / count as f64,
        down: down / count as f64,
    }
}

fn byte_balance(image: &[u8], base: &[u8], srgb: bool) -> Balance {
    let space = |v: u8| {
        if srgb {
            v as f64 / 255.0
        } else {
            srgb_decode(v as f32 / 255.0) as f64
        }
    };
    balance(
        image
            .chunks_exact(4)
            .zip(base.chunks_exact(4))
            .map(|(v, b)| (space(v[0]), space(b[0]))),
    )
}

fn mean_shift(image: &[u8], base: &[u8]) -> f64 {
    image
        .chunks_exact(4)
        .zip(base.chunks_exact(4))
        .map(|(v, b)| v[0] as f64 - b[0] as f64)
        .sum::<f64>()
        / (image.len() / 4) as f64
}

fn mean_stays_put(shift: f64) -> bool {
    shift.abs() <= 1.0
}

fn grain(amount: f32) -> Fx {
    Fx::grain(GrainParams {
        amount,
        ..Default::default()
    })
}

fn layer(amount: f32) -> FxLayer {
    grain(amount)
        .on(Shape::rounded(0.0, 0.0, SIZE as f32, SIZE as f32, 0.0))
        .seed(1)
        .at(1.0)
}

fn grain_on_grey(_: &(), _: &mut Cx<()>) -> El {
    el().size(SIZE as f32, SIZE as f32)
        .fill(hex("#808080").unwrap())
        .child(effect(1).absolute(0.0, 0.0).size(SIZE as f32, SIZE as f32))
}

fn half(bits: u16) -> f64 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = (bits >> 10) & 31;
    let fraction = (bits & 1023) as f64;
    sign * match exponent {
        0 => fraction * 2.0f64.powi(-24),
        31 if fraction == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        _ => (1024.0 + fraction) * 2.0f64.powi(exponent as i32 - 25),
    }
}

fn float_image(
    gpu: &Gpu,
    fx: &mut FxRenderer,
    srgb: bool,
    layers: &[FxLayer],
    slots: &[FxSlot],
) -> (Vec<[f64; 4]>, FxStats) {
    let format = wgpu::TextureFormat::Rgba16Float;
    let row_bytes = SIZE * 8;
    assert_eq!(row_bytes % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("grain symmetry"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("grain float readback"),
        size: u64::from(row_bytes * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let stats = fx.prepare_with_slots(
        &gpu.device,
        &gpu.queue,
        &FxFrame {
            viewport: [SIZE, SIZE],
            scale: 1.0,
            layers,
            backdrop: None,
            sources: &[],
        },
        slots,
    );
    let grey = if srgb {
        128.0 / 255.0
    } else {
        srgb_decode(128.0 / 255.0)
    } as f64;
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: grey,
                        g: grey,
                        b: grey,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        fx.draw_all(&mut pass, layers, None);
    }
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(SIZE),
            },
        },
        texture.size(),
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let pixels = {
        let data = slice.get_mapped_range().unwrap();
        data.chunks_exact(8)
            .map(|p| std::array::from_fn(|c| half(u16::from_le_bytes([p[c * 2], p[c * 2 + 1]]))))
            .collect()
    };
    buffer.unmap();
    (pixels, stats)
}

fn snapshot_names(srgb: bool) -> (&'static str, &'static str) {
    if srgb {
        ("grain-gamma", "grain-base-gamma")
    } else {
        ("grain-linear", "grain-base-linear")
    }
}

fn check(srgb: bool) {
    let (name, base_name) = snapshot_names(srgb);
    let formats = Arc::default();
    let (width, bytes) = snap_with(name, grain_on_grey, srgb, Arc::clone(&formats), 1, || {
        grain(AMOUNT)
    });
    let (base_width, base) = snap_with(
        base_name,
        grain_on_grey,
        srgb,
        Arc::clone(&formats),
        1,
        || grain(0.0),
    );
    let byte_signal = byte_balance(&bytes, &base, srgb);
    let shift = mean_shift(&bytes, &base);

    let gpu = Gpu::headless();
    eprintln!(
        "grain float adapter: {:?}, srgb {srgb}, format Rgba16Float",
        gpu.adapter.get_info()
    );
    let mut fx = FxRenderer::new(
        &gpu.device,
        wgpu::TextureFormat::Rgba16Float,
        FxOptions { dither: false },
    );
    fx.set_srgb_blend(srgb);
    let (empty, empty_stats) = float_image(&gpu, &mut fx, srgb, &[], &[]);
    let (zero, zero_stats) = float_image(&gpu, &mut fx, srgb, &[layer(0.0)], &[]);
    let (image, stats) = float_image(&gpu, &mut fx, srgb, &[layer(AMOUNT)], &[]);
    let (biased, biased_stats) = float_image(
        &gpu,
        &mut fx,
        srgb,
        &[layer(AMOUNT), layer(0.018)],
        &[FxSlot::default(), FxSlot::default().blend(FxBlend::Screen)],
    );
    let signal = balance(image.iter().zip(&zero).map(|(v, b)| (v[0], b[0])));
    let bias_signal = balance(biased.iter().zip(&zero).map(|(v, b)| (v[0], b[0])));
    eprintln!(
        "srgb {srgb}: byte {byte_signal:?}, mean shift {shift:.6}/255; float {signal:?}, imbalance {:.6}; biased float {bias_signal:?}, imbalance {:.6}",
        signal.imbalance(),
        bias_signal.imbalance()
    );

    assert_eq!((width, base_width), (SIZE, SIZE));
    assert_eq!(bytes.len(), (SIZE * SIZE * 4) as usize);
    assert_eq!(base.len(), bytes.len());
    let format = if srgb {
        wgpu::TextureFormat::Rgba8Unorm
    } else {
        wgpu::TextureFormat::Rgba8UnormSrgb
    };
    assert_eq!(formats.lock().unwrap().as_slice(), [format]);
    assert!(
        base.chunks_exact(4).all(|p| p == [128, 128, 128, 255]),
        "srgb {srgb}: amount-zero byte baseline"
    );
    assert!(
        bytes.chunks_exact(4).all(|p| p[3] == 255),
        "srgb {srgb}: byte opaque alpha"
    );
    assert!(
        byte_signal.visible(),
        "srgb {srgb}: byte grain should show: {byte_signal:?}"
    );
    assert!(byte_signal.imbalance() <= 0.2, "srgb {srgb}: byte grain imbalance: {byte_signal:?}");
    if srgb {
        assert!(
            mean_stays_put(shift),
            "srgb {srgb}: mean shifts by {shift:.2}/255"
        );
    }

    assert_eq!(
        (
            empty_stats.drawn,
            zero_stats.drawn,
            zero_stats.culled_idle,
            stats.drawn,
            biased_stats.drawn
        ),
        (0, 0, 1, 1, 2)
    );
    assert_eq!(
        empty, zero,
        "srgb {srgb}: amount-zero grain is culled and leaves the float baseline unchanged"
    );
    for pixels in [&empty, &zero, &image, &biased] {
        assert_eq!(pixels.len(), (SIZE * SIZE) as usize);
        assert!(
            pixels.iter().flatten().all(|v| v.is_finite()),
            "srgb {srgb}: float RGBA must be finite"
        );
        assert!(
            pixels
                .iter()
                .all(|p| p[..3].iter().all(|v| (0.0..=1.0).contains(v))),
            "srgb {srgb}: float RGB range"
        );
    }
    assert!(
        zero.iter()
            .all(|p| p == &zero[0] && p[..3].iter().all(|v| *v > 0.0 && *v < 1.0)),
        "srgb {srgb}: uniform float grey baseline"
    );
    assert!(
        signal.visible(),
        "srgb {srgb}: float grain should show: {signal:?}"
    );
    // Small UNORM sources skew byte symmetry; keep byte signal/mean checks and measure balance in float.
    assert!(
        signal.symmetric(),
        "srgb {srgb}: float grain must be symmetric: {signal:?}"
    );
    // Before storage rounding, Screen adds 28.2–30% to positive movement and leaves black unchanged.
    assert!(
        bias_signal.visible(),
        "srgb {srgb}: biased float grain should show: {bias_signal:?}"
    );
    assert!(
        !bias_signal.symmetric(),
        "srgb {srgb}: biased float grain must fail symmetry: {bias_signal:?}"
    );
}

#[test]
fn grain_is_symmetric_in_linear_blend() {
    check(false);
}

#[test]
fn grain_is_symmetric_in_gamma_blend() {
    check(true);
}

#[test]
fn grain_numerics() {
    let (linear, linear_base) = snapshot_names(false);
    let (gamma, gamma_base) = snapshot_names(true);
    let names = [linear, linear_base, gamma, gamma_base];
    for (i, name) in names.iter().enumerate() {
        assert!(
            !names[..i].contains(name),
            "grain snapshot names must be unique across modes"
        );
    }
    for (bits, expected) in [
        (0x0000, 0.0),
        (0x0001, 2.0f64.powi(-24)),
        (0x8001, -2.0f64.powi(-24)),
        (0x03ff, 1023.0 * 2.0f64.powi(-24)),
        (0x0400, 2.0f64.powi(-14)),
        (0x3800, 0.5),
        (0x3c00, 1.0),
        (0x3bff, 0.99951171875),
        (0x7bff, 65504.0),
        (0xfbff, -65504.0),
    ] {
        assert_eq!(half(bits), expected);
    }
    assert!(half(0x8000).is_sign_negative());
    assert_eq!(half(0x7c00), f64::INFINITY);
    assert_eq!(half(0xfc00), f64::NEG_INFINITY);
    assert!(half(0x7e00).is_nan());

    let base = [128, 128, 128, 255, 128, 128, 128, 255];
    for srgb in [false, true] {
        assert!(
            !byte_balance(&base, &base, srgb).visible(),
            "blank byte capture must fail visibility"
        );
    }
    let shifted = [129, 129, 129, 255, 130, 130, 130, 255];
    assert_eq!(mean_shift(&shifted, &base), 1.5);
    assert!(
        !mean_stays_put(mean_shift(&shifted, &base)),
        "+1.5 byte gamma drift must fail"
    );
    let blank = balance([(0.5, 0.5)].into_iter());
    assert!(!blank.visible());
    let good = balance([(0.51, 0.5), (0.49, 0.5)].into_iter());
    assert!(good.visible() && good.symmetric());
    let bad = balance([(0.515, 0.5), (0.49, 0.5)].into_iter());
    assert!(bad.visible() && !bad.symmetric());
}
