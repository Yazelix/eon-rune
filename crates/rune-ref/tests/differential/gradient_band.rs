//! Beta intentionally accepts recurrence of the frozen one-adjacent-value ownership switch.
//! The unchanged replay preserves evidence; unique-owner detection at those ambiguous pixels
//! returns with the post-beta product fix. Excluded domains keep the strict raw-policy result.

use super::*;
use rune::render::{Instance, LayerSpec};

#[test]
fn transparent_and_layered_linear_bands_keep_the_raw_policy() {
    let mut gpu = GpuRenderer::default();
    let mut scene = generate::scene(&[]);
    scene.clear = [0.0; 4];
    let lower = Primitive {
        node: 1,
        source: "gradient/lower".into(),
        rect: [17.25, 10.0, 32.0, 32.0],
        radii: [0.0; 4],
        fill: [0.25, 0.0, 0.0, 0.5],
        border: [0.25, 0.0, 0.0, 0.5],
        params: [0.0, 11.0, 0.0, 2.375],
        uv: [0.03125, 0.015625, 0.0, 0.0],
        clip: rune_ref::NO_CLIP,
        fade: [0.0; 4],
        fade_origin: None,
        turn: None,
        clip_radii: None,
    };
    let upper = Primitive {
        node: 2,
        source: "gradient/upper".into(),
        fill: [0.0, 0.25, 0.0, 0.5],
        border: [0.0, 0.25, 0.0, 0.5],
        params: [2.375, 11.0, 0.0, 1.0],
        ..lower.clone()
    };
    scene.primitives = vec![lower, upper];
    // Exact-owner assertions for this product case remain with the post-beta repair.
    // The beta exception must not hide its raw outliers in this excluded domain.
    for band in &scene.primitives {
        let mut single = scene.clone();
        single.primitives = vec![band.clone()];
        raw_policy(&mut gpu, &single);
    }
    let forward = raw_policy(&mut gpu, &scene);
    scene.primitives.reverse();
    assert_eq!(raw_policy(&mut gpu, &scene), forward);
    scene.primitives.reverse();
    // Opacity forces an isolated off-origin target rather than a flattened scope.
    let begin = Instance::begin_layer(
        [16.0, 8.0, 36.0, 36.0],
        rune_ref::NO_CLIP,
        LayerSpec {
            opacity: 0.75,
            ..Default::default()
        },
    );
    scene.primitives.insert(
        0,
        Primitive::from_instance(&begin, 3, "gradient/layer".into()),
    );
    scene.primitives.push(Primitive::from_instance(
        &Instance::end_layer(),
        4,
        "gradient/end".into(),
    ));
    raw_policy(&mut gpu, &scene);
}

fn raw_policy(gpu: &mut GpuRenderer, scene: &Scene) -> Vec<u8> {
    let mut cpu = Frame::new(scene.width, scene.height, true);
    cpu.render(scene);
    let actual = gpu.render(scene);
    let raw = compare(scene, &cpu, &actual, 2);
    let outcome = gpu.qualify(scene, &cpu, &actual, 2);
    assert_eq!(outcome.is_ok(), raw.outlier_pixels == 0);
    if let Ok(max) = outcome {
        assert_eq!(max, raw.max_channel);
    }
    eprintln!(
        "excluded-domain raw-policy: max={} outliers={}",
        raw.max_channel, raw.outlier_pixels
    );
    actual
}

#[test]
fn frozen_boundary_values_are_intentionally_qualified_for_beta() {
    let scene: Scene = serde_json::from_str(include_str!(
        "../../regressions/gradient-band-pixel-center.json"
    ))
    .unwrap();
    let mut gpu = GpuRenderer::default();
    let mut cpu = Frame::new(scene.width, scene.height, true);
    cpu.render(&scene);
    let actual = gpu.render(&scene);
    let raw = compare(&scene, &cpu, &actual, 2);
    eprintln!(
        "beta boundary waiver: raw_max={} raw_outliers={}",
        raw.max_channel, raw.outlier_pixels
    );
    assert_eq!(gpu.qualify(&scene, &cpu, &actual, 2), Ok(raw.max_channel));
}

#[test]
fn ambiguous_adjacent_bands_keep_a_shared_choice_in_both_orders() {
    let mut scene: Scene = serde_json::from_str(include_str!(
        "../../regressions/gradient-band-pixel-center.json"
    ))
    .unwrap();
    scene.primitives[0].fill = [0.25, 0.0, 0.0, 0.5];
    scene.primitives[0].border = scene.primitives[0].fill;
    let mut upper = scene.primitives[0].clone();
    upper.node += 1;
    upper.fill = [0.0, 0.25, 0.0, 0.5];
    upper.border = upper.fill;
    upper.params[0] = 2.375;
    upper.params[3] = 1.0;
    scene.primitives.push(upper);
    let mut gpu = GpuRenderer::default();
    check(&mut gpu, &scene, 2).expect("consistent boundary group");
    let forward = gpu.render(&scene);
    scene.primitives.reverse();
    check(&mut gpu, &scene, 2).expect("consistent reversed boundary group");
    assert_eq!(
        gpu.render(&scene),
        forward,
        "reversing disjoint owners keeps the result"
    );
}

fn partition(gpu: &mut GpuRenderer, scene: &Scene, samples: &[(u32, u32, usize)]) {
    assert_eq!(scene.primitives.len(), 2);
    let mut singles = Vec::new();
    for band in &scene.primitives {
        let mut single = scene.clone();
        single.primitives = vec![band.clone()];
        check(gpu, &single, 2).unwrap_or_else(|error| panic!("single gradient band: {error}"));
        singles.push(gpu.render(&single));
    }
    check(gpu, scene, 2).unwrap_or_else(|error| panic!("adjacent gradient bands: {error}"));
    let combined = gpu.render(scene);
    for &(x, y, owner) in samples {
        let at = ((y * scene.width + x) * 4) as usize;
        assert_eq!(
            singles[1 - owner][at..at + 4],
            [0; 4],
            "other band at {x},{y}"
        );
        assert_eq!(singles[owner][at + 3], 128, "owner at {x},{y}");
        assert_eq!(
            combined[at..at + 4],
            singles[owner][at..at + 4],
            "one owner at {x},{y}"
        );
    }
    let mut reversed = scene.clone();
    reversed.primitives.reverse();
    assert_eq!(
        gpu.render(&reversed),
        combined,
        "disjoint owners are order independent"
    );
}

#[test]
fn adjacent_radial_bands_own_exact_and_neighboring_centers() {
    let mut gpu = GpuRenderer::default();
    for center in [
        15.875_f32.to_bits() - 1,
        15.875_f32.to_bits(),
        15.875_f32.to_bits() + 1,
    ] {
        let mut scene = generate::scene(&[]);
        scene.clear = [0.0; 4];
        let lower = Primitive {
            node: 1,
            source: "radial/lower".into(),
            rect: [16.25, 16.25, 32.0, 32.0],
            radii: [0.0; 4],
            fill: [0.25, 0.0, 0.0, 0.5],
            border: [0.25, 0.0, 0.0, 0.5],
            params: [0.0, 11.0, 0.0, 2.375],
            uv: [f32::from_bits(center), 16.25, 1.0, 2.0],
            clip: rune_ref::NO_CLIP,
            fade: [0.0; 4],
            fade_origin: None,
            turn: None,
            clip_radii: None,
        };
        let upper = Primitive {
            node: 2,
            source: "radial/upper".into(),
            fill: [0.0, 0.25, 0.0, 0.5],
            border: [0.0, 0.25, 0.0, 0.5],
            params: [2.375, 11.0, 0.0, 1.0],
            ..lower.clone()
        };
        scene.primitives = vec![lower, upper];
        let owner = usize::from(center <= 15.875_f32.to_bits());
        partition(&mut gpu, &scene, &[(32, 32, owner)]);
    }
}

#[test]
fn cropped_runtime_radial_bands_with_fractional_geometry() {
    use rune::{Color, Cx, El, Env, Runtime, el};
    fn view(origin: &[f32; 2], _: &mut Cx<[f32; 2]>) -> El {
        el().size(128.0, 128.0).child(
            el().size(100.0, 80.0)
                .absolute(origin[0], origin[1])
                .radial_gradient(
                    [0.37, 0.41],
                    [37.0, 29.0],
                    [
                        (
                            0.0,
                            Color {
                                r: 0.5,
                                g: 0.0,
                                b: 0.0,
                                a: 0.5,
                            },
                        ),
                        (
                            0.375,
                            Color {
                                r: 0.0,
                                g: 0.5,
                                b: 0.0,
                                a: 0.5,
                            },
                        ),
                        (
                            1.0,
                            Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.5,
                                a: 0.5,
                            },
                        ),
                    ],
                ),
        )
    }
    let mut gpu = GpuRenderer::default();
    for origin in [[7.25, 9.25], [7.13, 9.17], [13.1, 11.3], [17.01, 8.03]] {
        let mut rt = Runtime::new(origin, view, (128.0, 128.0), Env::new(true, 1.0));
        rt.frame(0.0);
        let mut scene = generate::scene(&[]);
        scene.width = 128;
        scene.height = 128;
        scene.clear = [0.0; 4];
        scene.primitives = rt
            .instances
            .iter()
            .filter(|p| p.params[1] == 11.0)
            .enumerate()
            .map(|(n, p)| Primitive::from_instance(p, n as u32 + 1, format!("cropped/{n}")))
            .collect();
        assert_eq!(scene.primitives.len(), 2);
        assert_ne!(
            scene.primitives[0].rect, scene.primitives[1].rect,
            "must exercise cropping"
        );
        let frames: Vec<_> = scene
            .primitives
            .iter()
            .map(|p| {
                let mut single = scene.clone();
                single.primitives = vec![p.clone()];
                check(&mut gpu, &single, 2).expect("cropped radial segment");
                gpu.render(&single)
            })
            .collect();
        for y in 12..80 {
            for x in 20..100 {
                let a = ((y * scene.width + x) * 4 + 3) as usize;
                assert!(
                    frames[0][a] == 0 || frames[1][a] == 0,
                    "overlapping cropped bands at {x},{y}"
                );
                assert_eq!(
                    u16::from(frames[0][a]) + u16::from(frames[1][a]),
                    128,
                    "cropped bands leave a hole at {x},{y}"
                );
            }
        }
        check(&mut gpu, &scene, 2).expect("cropped radial segments");
    }
}
