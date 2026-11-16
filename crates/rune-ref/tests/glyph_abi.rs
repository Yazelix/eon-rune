#![cfg(feature = "gpu")]

use rune::render::Instance;
use rune_ref::{Primitive, Scene, generate};

fn saved_glyph() -> Primitive {
    let scene: Scene =
        serde_json::from_str(include_str!("../regressions/glyph-texel-boundary.json")).unwrap();
    scene.primitives.into_iter().next().unwrap()
}

#[test]
fn portable_glyph_lowers_to_a_page_zero_mask_without_changing_its_scene() {
    let glyph = saved_glyph();
    let before = serde_json::to_vec(&glyph).unwrap();
    let instance = Instance::from(&glyph);
    assert_eq!(instance.radii, [0.0; 4]);
    assert_eq!(
        instance.params,
        [glyph.params[0], 2.0, 0.0, glyph.params[3]]
    );
    assert_eq!(instance.rect, glyph.rect);
    assert_eq!(instance.uv, glyph.uv);
    assert_eq!(instance.fill, glyph.fill);
    assert_eq!(instance.border, glyph.border);
    assert_eq!(instance.clip, glyph.clip);
    let capture = Primitive::from_instance(&instance, glyph.node, glyph.source.clone());
    assert_eq!(capture.node, 4);
    assert_eq!(capture.source, "generated/3");
    assert_eq!(capture.fade, glyph.fade);
    assert_eq!(Instance::from(&capture).fade, instance.fade);
    assert_eq!(serde_json::to_vec(&glyph).unwrap(), before);
}

#[test]
fn production_glyph_capture_rejects_unsupported_representations() {
    let valid = Instance::from(&saved_glyph());
    let mut unsupported = Vec::new();
    for corner in 0..4 {
        let mut instance = valid;
        instance.radii[corner] = 1.0;
        unsupported.push(instance);
    }
    for page in [0.125, 1.0, f32::NAN] {
        let mut instance = valid;
        instance.params[2] = page;
        unsupported.push(instance);
    }
    for exponent in [0.0, -1.0, f32::NAN] {
        let mut instance = valid;
        instance.params[3] = exponent;
        unsupported.push(instance);
    }
    for instance in unsupported {
        let error = std::panic::catch_unwind(|| {
            Primitive::from_instance(&instance, 42, "capture/glyph".into())
        })
        .expect_err("unsupported production text must not become an atlas-mask replay");
        let message = error.downcast_ref::<String>().unwrap();
        assert!(message.contains("42 (capture/glyph)"), "{message}");
    }
}

#[test]
fn non_glyph_conversion_keeps_its_radii_and_params() {
    for primitive in generate::radial(1, 8).primitives {
        let instance = Instance::from(&primitive);
        let capture = Primitive::from_instance(&instance, primitive.node, primitive.source.clone());
        assert_eq!(capture.radii, primitive.radii);
        assert_eq!(capture.params, primitive.params);
        assert_eq!(capture.uv, primitive.uv);
        assert_eq!(capture.fill, primitive.fill);
        assert_eq!(capture.border, primitive.border);
    }
}
