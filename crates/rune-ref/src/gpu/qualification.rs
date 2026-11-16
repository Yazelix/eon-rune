//! Empirical framebuffer qualification; canonical rendering and raw comparison stay independent.
use super::{GpuRenderer, OFFSCREEN_FORMAT};
use crate::{Frame, Scene, compare};

#[cfg(test)]
mod tests;

fn measured(
    backend: &str,
    vendor: u32,
    device: u32,
    name: &str,
    driver: &str,
    build: &str,
    shader: &str,
) -> bool {
    shader == "windfoil (compiled)"
        && backend == "vulkan"
        && matches!(
            (vendor, device, name, driver, build),
            (
                0x8086,
                0xa78b,
                "Intel(R) Graphics (RPL-S)",
                "Intel open-source Mesa driver",
                "Mesa 26.1.6-1pop0~1787580452~24.04~a5619ea"
            ) | (
                0x10005,
                0,
                "llvmpipe (LLVM 20.1.2, 256 bits)",
                "llvmpipe",
                "Mesa 26.1.6-1pop0~1787580452~24.04~a5619ea (LLVM 20.1.2)"
            )
        )
}

fn measured_renderer(gpu: &GpuRenderer) -> bool {
    let info = gpu.gpu.adapter.get_info();
    measured(
        info.backend.to_str(),
        info.vendor,
        info.device,
        &info.name,
        &info.driver,
        &info.driver_info,
        gpu.renderer.uber_shader(),
    )
}

fn eligible(scene: &Scene, tolerance: u8, linear_srgb_target: bool) -> bool {
    tolerance == 2
        && linear_srgb_target
        && scene.width == 64
        && scene.height == 64
        && scene.clear[3] == 1.0
        && scene.primitives.iter().all(|p| {
            matches!(p.params[1], 0.0 | 1.0 | 2.0 | 3.0 | 4.0 | 12.0)
                || (p.params[1] == 11.0 && p.uv[2] > 0.0 && p.uv[3] > 0.0)
        })
}

fn pixel(bytes: &[u8], at: usize) -> [u8; 4] {
    bytes.as_chunks::<4>().0[at]
}

fn guard(
    before: [u8; 4],
    gpu_before: [u8; 4],
    reference: [u8; 4],
    predicted: [u8; 4],
    actual: [u8; 4],
    active: bool,
) -> Option<(&'static str, usize)> {
    if actual[3] != reference[3] {
        return Some(("alpha", 3));
    }
    for c in 0..3 {
        if !active {
            if actual[c] != gpu_before[c] {
                return Some(("no-op", c));
            }
        } else {
            if actual[c].abs_diff(predicted[c]) > 1 {
                return Some(("fresh", c));
            }
            if predicted[c].abs_diff(reference[c]) > gpu_before[c].abs_diff(before[c]) {
                return Some(("propagation", c));
            }
        }
    }
    None
}

impl GpuRenderer {
    /// Qualify differential test output on the measured framebuffer paths in research/24.
    ///
    /// The CPU frame and actual final bytes must be for `scene`. Other adapters/scenes use
    /// raw comparison. Eligible scenes check every prefix, even if the final raw delta fits.
    /// Returns the raw maximum, not a local allowance. Canonical bytes and `compare` are unchanged.
    pub fn qualify(
        &mut self,
        scene: &Scene,
        cpu: &Frame,
        actual: &[u8],
        tolerance: u8,
    ) -> Result<u8, String> {
        scene.validate()?;
        assert_eq!((cpu.width, cpu.height), (scene.width, scene.height));
        let raw = compare(scene, cpu, actual, tolerance);
        if !measured_renderer(self)
            || !eligible(
                scene,
                tolerance,
                OFFSCREEN_FORMAT.is_srgb() && !self.renderer.srgb_blend(),
            )
        {
            return if raw.outlier_pixels == 0 {
                Ok(raw.max_channel)
            } else {
                Err(serde_json::to_string(&raw).map_err(|error| error.to_string())?)
            };
        }
        qualify_prefixes(scene, cpu, actual, raw, |prefix| self.render(prefix))
    }
}

fn qualify_prefixes(
    scene: &Scene,
    cpu: &Frame,
    actual: &[u8],
    raw: crate::Difference,
    mut render: impl FnMut(&Scene) -> Vec<u8>,
) -> Result<u8, String> {
    let mut prefix = scene.clone();
    prefix.primitives.clear();
    let mut reference = Frame::new(scene.width, scene.height, false);
    reference.render(&prefix);
    let mut gpu_before = if scene.primitives.is_empty() {
        actual.to_vec()
    } else {
        render(&prefix)
    };
    if let Some(at) = gpu_before
        .iter()
        .zip(&reference.rgba)
        .position(|(a, b)| a != b)
    {
        return Err(format!(
            "framebuffer clear differs at ({},{}), channel {}",
            at / 4 % scene.width as usize,
            at / 4 / scene.width as usize,
            at % 4
        ));
    }
    let pixels = (scene.width * scene.height) as usize;
    let mut owners = vec![None; pixels];
    let mut operations = vec![0u32; pixels];
    let mut before = reference.rgba.clone();
    let mut predicted = reference.rgba.clone();
    for (index, primitive) in scene.primitives.iter().enumerate() {
        prefix.primitives.push(primitive.clone());
        before.copy_from_slice(&reference.rgba);
        owners.fill(None);
        Frame::render_range(
            scene.width,
            scene.height,
            &mut reference.rgba,
            Some(&mut owners),
            scene,
            index,
            index + 1,
        );
        predicted.copy_from_slice(&gpu_before);
        Frame::render_range(
            scene.width,
            scene.height,
            &mut predicted,
            None,
            scene,
            index,
            index + 1,
        );
        let current = if index + 1 == scene.primitives.len() {
            actual.to_vec()
        } else {
            render(&prefix)
        };
        for at in 0..pixels {
            let active = owners[at].is_some();
            operations[at] += u32::from(active);
            let r = pixel(&reference.rgba, at);
            let g = pixel(&current, at);
            if let Some((failed, channel)) = guard(
                pixel(&before, at),
                pixel(&gpu_before, at),
                r,
                pixel(&predicted, at),
                g,
                active,
            ) {
                return Err(serde_json::json!({
                    "guard": failed,
                    "prefix": index + 1,
                    "x": at as u32 % scene.width,
                    "y": at as u32 / scene.width,
                    "channel": channel,
                    "node": primitive.node,
                    "source": primitive.source,
                    "canonical_before": pixel(&before, at),
                    "actual_before": pixel(&gpu_before, at),
                    "canonical": r,
                    "predicted": pixel(&predicted, at),
                    "actual": g,
                    "operations": operations[at],
                    "derived_rgb_bound": operations[at],
                    "raw_difference": raw
                })
                .to_string());
            }
            debug_assert!((0..3).all(|c| u32::from(g[c].abs_diff(r[c])) <= operations[at]));
        }
        gpu_before = current;
    }
    if reference.rgba != cpu.rgba {
        return Err("CPU frame does not match scene".into());
    }
    if raw.outlier_pixels > 0 {
        eprintln!(
            "framebuffer-qualified: raw_max={} raw_outliers={} max_operations={}",
            raw.max_channel,
            raw.outlier_pixels,
            operations.iter().max().unwrap_or(&0)
        );
    }
    Ok(raw.max_channel)
}
