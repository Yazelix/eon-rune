use super::*;

#[test]
fn local_error_cannot_borrow_an_overlap_allowance() {
    let pixel = [80, 90, 100, 255];
    for channel in 0..3 {
        for sign in [-1i16, 1] {
            let mut actual = pixel;
            actual[channel] = (i16::from(pixel[channel]) + sign) as u8;
            assert_eq!(guard(pixel, pixel, pixel, pixel, actual, true), None);
            actual[channel] = (i16::from(pixel[channel]) + sign * 2) as u8;
            assert_eq!(
                guard(pixel, pixel, pixel, pixel, actual, true),
                Some(("fresh", channel))
            );
        }
    }
}

#[test]
fn no_op_alpha_and_propagation_are_distinct_failures() {
    let reference = [80, 90, 100, 255];
    let mut changed = reference;
    changed[0] += 1;
    assert_eq!(
        guard(reference, reference, reference, reference, changed, false),
        Some(("no-op", 0))
    );
    changed = reference;
    changed[3] -= 1;
    assert_eq!(
        guard(reference, reference, reference, reference, changed, true),
        Some(("alpha", 3))
    );
    changed = reference;
    changed[0] += 2;
    assert_eq!(
        guard(reference, reference, reference, changed, changed, true),
        Some(("propagation", 0))
    );
}

#[test]
fn profile_changes_and_excluded_scenes_keep_the_raw_policy() {
    for (vendor, device, mut fields) in [
        (
            0x8086,
            0xa78b,
            [
                "vulkan",
                "Intel(R) Graphics (RPL-S)",
                "Intel open-source Mesa driver",
                "Mesa 26.1.6-1pop0~1787580452~24.04~a5619ea",
                "windfoil (compiled)",
            ],
        ),
        (
            0x10005,
            0,
            [
                "vulkan",
                "llvmpipe (LLVM 20.1.2, 256 bits)",
                "llvmpipe",
                "Mesa 26.1.6-1pop0~1787580452~24.04~a5619ea (LLVM 20.1.2)",
                "windfoil (compiled)",
            ],
        ),
    ] {
        assert!(measured(
            fields[0], vendor, device, fields[1], fields[2], fields[3], fields[4]
        ));
        for at in 0..fields.len() {
            let old = fields[at];
            fields[at] = "unmeasured";
            assert!(!measured(
                fields[0], vendor, device, fields[1], fields[2], fields[3], fields[4]
            ));
            fields[at] = old;
        }
        assert!(!measured(
            fields[0],
            vendor ^ 1,
            device,
            fields[1],
            fields[2],
            fields[3],
            fields[4]
        ));
        assert!(!measured(
            fields[0],
            vendor,
            device ^ 1,
            fields[1],
            fields[2],
            fields[3],
            fields[4]
        ));
    }
    let base = crate::generate::seeded(0, 1);
    assert!(eligible(&base, 2, true));
    for tolerance in [0, 1, 3] {
        assert!(!eligible(&base, tolerance, true));
    }
    assert!(!eligible(&base, 2, false));
    let mut scene = base.clone();
    scene.width = 65;
    assert!(!eligible(&scene, 2, true));
    scene = base.clone();
    scene.height = 65;
    assert!(!eligible(&scene, 2, true));
    scene = base.clone();
    scene.clear = [0.; 4];
    assert!(!eligible(&scene, 2, true));
    for kind in [8., 9., 10.] {
        scene = base.clone();
        scene.primitives[0].params[1] = kind;
        assert!(!eligible(&scene, 2, true));
    }
    scene = base.clone();
    scene.primitives[0].params[1] = 11.;
    scene.primitives[0].uv = [0.5, 0.0, 0.0, 0.0];
    assert!(
        !eligible(&scene, 2, true),
        "angled-linear gradients stay on the raw comparison"
    );
    scene.primitives[0].uv = [8., 8., 16., 16.];
    assert!(
        eligible(&scene, 2, true),
        "positive radial extents are measured"
    );
    let mut layer = base.primitives[0].clone();
    layer.params = [1., 8., 0., 0.];
    layer.radii = [0.; 4];
    layer.border = [0.; 4];
    layer.fill = [0., 0., 0., 1.];
    layer.uv = [0.; 4];
    layer.clip = crate::NO_CLIP;
    let mut end = layer.clone();
    end.params = [0., 9., 0., 0.];
    scene = base.clone();
    scene.primitives = vec![layer, base.primitives[0].clone(), end];
    scene.validate().expect("valid additive layer");
    assert!(!eligible(&scene, 2, true));
}

#[test]
fn wrong_blends_and_propagation_bounds_use_the_shared_guard() {
    let source = [0.08, 0.04, 0.02, 0.25];
    let destination = [0.12, 0.18, 0.24, 1.0];
    let correct = crate::encode(std::array::from_fn(|c| {
        source[c] + destination[c] * (1. - source[3])
    }));
    let before = crate::encode(destination);
    let additive = crate::encode(std::array::from_fn(|c| source[c] + destination[c]));
    let straight = crate::encode(std::array::from_fn(|c| {
        if c == 3 {
            1.
        } else {
            source[c] * source[3] + destination[c] * (1. - source[3])
        }
    }));
    for wrong in [additive, straight] {
        assert!((0..3).any(|c| wrong[c].abs_diff(correct[c]) > 1));
        assert!(matches!(
            guard(before, before, correct, correct, wrong, true),
            Some(("fresh", _))
        ));
    }
    for prior in -3i16..=3 {
        for propagated in -prior.abs()..=prior.abs() {
            for fresh in [-1i16, 0, 1] {
                let mut old = correct;
                old[0] = (i16::from(correct[0]) + prior) as u8;
                let mut predicted = correct;
                predicted[0] = (i16::from(correct[0]) + propagated) as u8;
                let mut actual = predicted;
                actual[0] = (i16::from(predicted[0]) + fresh) as u8;
                assert_eq!(guard(correct, old, correct, predicted, actual, true), None);
                assert!(i16::from(actual[0].abs_diff(correct[0])) <= prior.abs() + 1);
            }
        }
    }
}

#[test]
fn production_rendered_color_defect_cannot_use_the_overlap_count() {
    let mut gpu = GpuRenderer::default();
    let scene = opaque_scene(3);
    let mut cpu = Frame::new(64, 64, true);
    cpu.render(&scene);
    let actual = gpu.render(&scene);
    assert_eq!(
        gpu.qualify(&scene, &cpu, &actual, 2),
        Ok(compare(&scene, &cpu, &actual, 2).max_channel)
    );
    let mut broken = scene.clone();
    broken.primitives[2].fill = crate::decode([130, 128, 128, 255]);
    let wrong = gpu.render(&broken);
    let delta = wrong[0].abs_diff(cpu.rgba[0]);
    let result = gpu.qualify(&scene, &cpu, &wrong, 2);
    eprintln!(
        "wrong-color control: canonical={:?} original={:?} changed={:?} raw_max={} result={result:?}",
        &cpu.rgba[..4],
        &actual[..4],
        &wrong[..4],
        compare(&scene, &cpu, &wrong, 2).max_channel
    );
    if measured_renderer(&gpu) {
        assert!(
            (2..=3).contains(&delta),
            "wrong-color stimulus must fit count3: {delta}"
        );
        let error = result.expect_err("one draw cannot borrow the three-draw allowance");
        let report: serde_json::Value = serde_json::from_str(&error).expect("guard JSON");
        assert_eq!(report["guard"], "fresh");
        assert_eq!(report["prefix"], 3);
        assert_eq!(report["operations"], 3);
    } else {
        assert_eq!(
            result.is_ok(),
            compare(&scene, &cpu, &wrong, 2).outlier_pixels == 0
        );
    }
    let mut empty = scene.clone();
    empty.primitives.clear();
    cpu.render(&empty);
    let mut changed_clear = gpu.render(&empty);
    changed_clear[0] += 1;
    let result = gpu.qualify(&empty, &cpu, &changed_clear, 2);
    if measured_renderer(&gpu) {
        assert!(result.expect_err("exact clear").contains("clear differs"));
    } else {
        assert_eq!(
            result.is_ok(),
            compare(&empty, &cpu, &changed_clear, 2).outlier_pixels == 0
        );
    }
}

fn opaque_scene(draws: usize) -> Scene {
    let mut scene = crate::generate::seeded(0, 1);
    let mut p = scene.primitives[0].clone();
    p.rect = [0., 0., 64., 64.];
    p.radii = [0.; 4];
    p.fill = crate::decode([128, 128, 128, 255]);
    p.border = [0.; 4];
    p.params = [0., 0., 0.5, 1.];
    p.uv = [0.; 4];
    p.clip = crate::NO_CLIP;
    p.fade = [0.; 4];
    p.fade_origin = None;
    p.turn = None;
    scene.primitives = vec![p; draws];
    scene
}

#[test]
fn hidden_intermediate_defect_is_rejected_by_the_prefix_loop() {
    let scene = opaque_scene(3);
    let mut cpu = Frame::new(64, 64, true);
    cpu.render(&scene);
    let final_image = cpu.rgba.clone();
    let raw = compare(&scene, &cpu, &final_image, 2);
    assert_eq!(raw.max_channel, 0);
    let mut visited = Vec::new();
    let result = qualify_prefixes(&scene, &cpu, &final_image, raw, |prefix| {
        let count = prefix.primitives.len();
        visited.push(count);
        let mut frame = Frame::new(64, 64, false);
        frame.render(prefix);
        if count == 2 {
            frame.rgba[0] += 2;
        }
        frame.rgba
    });
    let error = result.expect_err("a later opaque draw must not hide an intermediate defect");
    let report: serde_json::Value = serde_json::from_str(&error).expect("guard JSON");
    assert_eq!(report["guard"], "fresh");
    assert_eq!(report["prefix"], 2);
    assert_eq!(report["actual"][0], 130);
    assert_eq!(report["predicted"][0], 128);
    assert_eq!(visited, vec![0, 1, 2]);
    assert_eq!(compare(&scene, &cpu, &final_image, 2).max_channel, 0);
}

#[test]
fn recorded_outliers_still_exercise_measured_qualification() {
    let mut gpu = GpuRenderer::default();
    let scenes: [Scene; 3] = [
        serde_json::from_str(include_str!(
            "../../../tests/framebuffer_precision/fixtures/intel-minimized.json"
        ))
        .expect("Intel scene"),
        serde_json::from_str(include_str!(
            "../../../tests/framebuffer_precision/fixtures/lvp-minimized.json"
        ))
        .expect("lavapipe scene"),
        crate::generate::radial(57, 8),
    ];
    for (index, scene) in scenes.into_iter().enumerate() {
        let mut cpu = Frame::new(64, 64, true);
        cpu.render(&scene);
        let actual = gpu.render(&scene);
        let raw = compare(&scene, &cpu, &actual, 2);
        if measured_renderer(&gpu) {
            if gpu.gpu.adapter.get_info().vendor == 0x10005 || index == 0 {
                assert!(
                    raw.outlier_pixels > 0,
                    "recorded witness must still exercise qualification"
                );
                assert_eq!(raw.max_channel, 3);
            }
            assert_eq!(gpu.qualify(&scene, &cpu, &actual, 2), Ok(raw.max_channel));
        } else {
            assert_eq!(
                gpu.qualify(&scene, &cpu, &actual, 2).is_ok(),
                raw.outlier_pixels == 0
            );
        }
    }
}
