use super::*;
use crate::compare;

fn frozen() -> Scene {
    serde_json::from_str(include_str!(
        "../../../../regressions/gradient-band-pixel-center.json"
    ))
    .unwrap_or_else(|error| panic!("frozen Scene: {error}"))
}

fn pair() -> Scene {
    let mut scene = frozen();
    let mut lower = scene.primitives[0].clone();
    lower.fill = [0.25, 0.0, 0.0, 0.5];
    lower.border = lower.fill;
    let mut upper = lower.clone();
    upper.node += 1;
    upper.fill = [0.0, 0.25, 0.0, 0.5];
    upper.border = upper.fill;
    upper.params[0] = 2.375;
    upper.params[3] = 1.0;
    scene.primitives = vec![lower, upper];
    scene
}

fn frames(scene: &Scene, mut include: impl FnMut(usize, usize, Decision) -> bool) -> Vec<Vec<u8>> {
    let mut prefix = scene.clone();
    prefix.primitives.clear();
    let mut frame = Frame::new(scene.width, scene.height, false);
    frame.render(&prefix);
    let model =
        Qualification::new(scene, &frame.rgba).unwrap_or_else(|| panic!("eligible test scene"));
    let mut result = vec![frame.rgba.clone()];
    for index in 0..scene.primitives.len() {
        let before = frame.rgba.clone();
        apply(scene, index, &mut frame.rgba, None);
        let mut on = before.clone();
        apply(&model.included, index, &mut on, None);
        for (at, decision) in model.ambiguity.decisions[index].iter().enumerate() {
            if let Some(decision) = decision {
                let source = if include(index, at, *decision) {
                    &on
                } else {
                    &before
                };
                frame.rgba[at * 4..at * 4 + 4].copy_from_slice(&source[at * 4..at * 4 + 4]);
            }
        }
        result.push(frame.rgba.clone());
    }
    result
}

fn qualify(scene: &Scene, frames: &[Vec<u8>]) -> Result<u8, String> {
    let mut cpu = Frame::new(scene.width, scene.height, true);
    cpu.render(scene);
    let actual = &frames[scene.primitives.len()];
    let raw = compare(scene, &cpu, actual, 2);
    super::super::qualify_prefixes(scene, &cpu, actual, raw, |prefix| {
        frames[prefix.primitives.len()].clone()
    })
}

#[test]
fn original_coordinates_intentionally_accept_both_frozen_ownership_values() {
    // Beta waiver: this keeps the evidence, not recurrence detection. The post-beta
    // product repair must restore a unique owner and its regression check.
    let scene = frozen();
    let ambiguity = Ambiguity::new(&scene);
    let coordinates: Vec<_> = ambiguity.decisions[0]
        .iter()
        .enumerate()
        .filter_map(|(at, value)| value.map(|_| (at % 64, at / 64)))
        .collect();
    assert_eq!(coordinates, (27..32).map(|y| (38, y)).collect::<Vec<_>>());
    for below in [false, true] {
        let images = frames(&scene, |_, _, d| below == d.include_below);
        assert!(qualify(&scene, &images).is_ok());
        let mut canonical = Frame::new(64, 64, true);
        canonical.render(&scene);
        assert!(compare(&scene, &canonical, &images[1], 2).outlier_pixels > 0);
    }
}

#[test]
fn hole_and_double_paint_are_rejected_in_both_orders() {
    let original = pair();
    for below in [false, true] {
        let forward = frames(&original, |_, _, d| below == d.include_below);
        assert!(qualify(&original, &forward).is_ok());
        let mut reversed = original.clone();
        reversed.primitives.reverse();
        let backward = frames(&reversed, |_, _, d| below == d.include_below);
        assert!(qualify(&reversed, &backward).is_ok());
        assert_eq!(forward[2], backward[2], "one owner is order independent");
    }
    for reversed in [false, true] {
        let mut scene = original.clone();
        if reversed {
            scene.primitives.reverse();
        }
        for paint in [false, true] {
            let images = frames(&scene, |_, _, _| paint);
            let error = qualify(&scene, &images).expect_err("hole/double-paint mutation");
            assert!(error.contains("boundary-owner"), "{error}");
        }
    }
}

#[test]
fn a_later_duplicate_band_cannot_change_the_group_choice() {
    let mut scene = pair();
    let mut duplicate = scene.primitives[1].clone();
    duplicate.node += 1;
    duplicate.fill = [0.0, 0.0, 0.25, 0.5];
    duplicate.border = duplicate.fill;
    scene.primitives.push(duplicate);
    let good = frames(&scene, |_, _, d| !d.include_below);
    assert!(qualify(&scene, &good).is_ok());
    let wrong = frames(&scene, |index, _, d| {
        if index == 2 {
            d.include_below
        } else {
            !d.include_below
        }
    });
    let error = qualify(&scene, &wrong).expect_err("inconsistent group mutation");
    assert!(
        error.contains("boundary-owner") && error.contains("\"prefix\":3"),
        "{error}"
    );
}

#[test]
fn image_corruption_cannot_change_the_independent_ambiguity_set() {
    let scene = pair();
    let expected = Ambiguity::new(&scene);
    let original = frames(&scene, |_, _, d| d.canonical_below == d.include_below);
    for target in 0..original.len() {
        let mut corrupted = original.clone();
        let mut seed = 0x715e53a9u32;
        for byte in &mut corrupted[target] {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *byte = seed as u8;
        }
        assert!(qualify(&scene, &corrupted).is_err());
        assert_eq!(Ambiguity::new(&scene), expected);
        let mut state = Qualification::new(&scene, &original[0]).unwrap();
        let before = state.ambiguity.clone();
        let inclusion = state
            .step(&scene, 0, &original[0], &corrupted[target])
            .unwrap();
        for at in 0..4096 {
            let _attempted_choice = state.choose(
                0,
                at,
                pixel(&original[0], at),
                pixel(&corrupted[target], at),
                pixel(&corrupted[target], at),
                &inclusion,
            );
        }
        assert_eq!(
            state.ambiguity, before,
            "choice attempts cannot mutate the set"
        );
    }
}

#[test]
fn wrong_color_alpha_and_pixels_outside_the_set_still_fail() {
    let scene = pair();
    let good = frames(&scene, |_, _, d| d.include_below);
    for (at, color) in [
        (0, [255, 0, 255, 255]),
        (27 * 64 + 38, [255, 0, 255, 255]),
        (27 * 64 + 38, [50, 60, 70, 254]),
    ] {
        let mut wrong = good.clone();
        wrong[1][at * 4..at * 4 + 4].copy_from_slice(&color);
        assert!(qualify(&scene, &wrong).is_err());
    }
}

#[test]
fn dyadic_exact_equality_has_no_boundary_exception() {
    let mut scene = pair();
    for p in &mut scene.primitives {
        p.rect = [17.25, 10.0, 32.0, 32.0];
        p.uv = [0.03125, 0.015625, 0.0, 0.0];
    }
    let model = Ambiguity::new(&scene);
    assert!(model.decisions.iter().flatten().all(Option::is_none));
    let mut images = frames(&scene, |_, _, _| false);
    let mut closed = scene.clone();
    closed.primitives[0].params[3] = 0.375;
    let mut on = images[0].clone();
    apply(&closed, 0, &mut on, None);
    let at = (23 * 64 + 30) * 4;
    images[1][at..at + 4].copy_from_slice(&on[at..at + 4]);
    assert!(
        qualify(&scene, &images).is_err(),
        "wrong equality ownership is not waived here"
    );
}

#[test]
fn excluded_domains_and_zero_alpha_do_not_gain_an_allowance() {
    let original = pair();
    assert!(eligible(&original));
    for mutate in 0..6 {
        let mut scene = original.clone();
        match mutate {
            0 => scene.width = 65,
            1 => scene.clear[3] = 0.0,
            2 => scene.primitives = vec![scene.primitives[0].clone(); 17],
            3 => scene.primitives[0].turn = Some([0.0; 3]),
            4 => scene.primitives[0].uv[3] = 1.0,
            _ => scene.primitives[0].params[3] = 2.0,
        }
        assert!(!eligible(&scene), "excluded domain {mutate}");
    }
    let mut empty = original;
    for p in &mut empty.primitives {
        p.fill = [0.0; 4];
        p.border = [0.0; 4];
    }
    let images = frames(&empty, |_, _, d| d.include_below);
    assert!(qualify(&empty, &images).is_ok());
    assert!(images.iter().all(|image| *image == images[0]));
    let mut wrong = images;
    wrong[1][(27 * 64 + 38) * 4] += 1;
    assert!(
        qualify(&empty, &wrong).is_err(),
        "an invisible band is an exact no-op"
    );
}
