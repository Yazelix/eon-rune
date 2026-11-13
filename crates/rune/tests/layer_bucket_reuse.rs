use rune::render::{
    Gpu, Instance, KIND_RECT, LayerSpec, NO_CLIP, OFFSCREEN_FORMAT, Offscreen, Renderer,
};

fn group(width: f32) -> Vec<Instance> {
    let bounds = [0.0, 0.0, width, 30.0];
    vec![
        Instance::begin_layer(
            bounds,
            NO_CLIP,
            LayerSpec {
                opacity: 0.5,
                ..Default::default()
            },
        ),
        Instance {
            rect: bounds,
            radii: [0.0; 4],
            fill: [1.0, 0.0, 0.0, 1.0],
            border: [0.0; 4],
            params: [0.0, KIND_RECT, 0.0, 1.0],
            uv: [0.0; 4],
            clip: NO_CLIP,
            fade: [0; 4],
        },
        Instance::end_layer(),
    ]
}

#[test]
fn header_shrink_reuses_two_buckets_but_replaces_a_larger_target() {
    let gpu = Gpu::headless();
    let mut renderer = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
    let mut atlas = rune::text::TextSystem::default().atlas;
    let target = Offscreen::new(&gpu.device, 256, 64);
    let mut draw = |renderer: &mut Renderer, width| {
        renderer.prepare(
            &gpu.device,
            &gpu.queue,
            &mut atlas,
            &group(width),
            (256, 64),
        );
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        renderer.draw(&mut encoder, &target.view, [0.0; 4]);
        target.copy(&mut encoder);
        renderer.submit(&gpu.queue, encoder);
        target.read(&gpu.device)
    };
    draw(&mut renderer, 120.0); // 128×64 allocation.
    let before = renderer.layer_stats().allocations;
    let pixels = draw(&mut renderer, 56.0); // 64×64 bucket; reuse its 2× predecessor.
    assert_eq!(renderer.layer_stats().allocations, before);
    assert_eq!(renderer.layer_stats().bytes, 128 * 64 * 8);
    let inside = &pixels[(10 * 256 + 20) * 4..(10 * 256 + 21) * 4];
    assert!(
        inside
            .iter()
            .zip([188, 0, 0, 128])
            .all(|(&a, b)| a.abs_diff(b) <= 1),
        "{inside:?}"
    );
    assert_eq!(&pixels[(10 * 256 + 80) * 4..(10 * 256 + 81) * 4], &[0; 4]);

    draw(&mut renderer, 256.0); // More than 2× the small bucket must be released.
    let before = renderer.layer_stats().allocations;
    draw(&mut renderer, 56.0);
    assert_eq!(renderer.layer_stats().allocations, before + 1);
    assert_eq!(renderer.layer_stats().bytes, 64 * 64 * 8);
}

#[test]
fn shrinking_one_cache_can_admit_a_second_without_exceeding_budget() {
    fn view(s: &(f32, bool), _: &mut rune::Cx<(f32, bool)>) -> rune::El {
        let mut body = rune::row().size(256.0, 64.0).child(
            rune::el()
                .key("a")
                .size(s.0, 30.0)
                .cache()
                .fill(rune::Color::WHITE),
        );
        if s.1 {
            body = body.child(
                rune::el()
                    .key("b")
                    .size(56.0, 30.0)
                    .cache()
                    .fill(rune::Color::WHITE),
            );
        }
        body
    }
    let gpu = Gpu::headless();
    let mut renderer = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
    renderer.set_cache_budget(64 * 1024);
    let mut rt = rune::Runtime::new(
        (120.0, false),
        view,
        (256.0, 64.0),
        rune::Env::new(true, 1.0),
    );
    rt.frame(0.0);
    renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &mut rt.text.atlas,
        &rt.instances,
        (256, 64),
    );
    assert_eq!(renderer.layer_stats().bytes, 64 * 1024);
    rt.update(|s, tx| {
        *s = (56.0, true);
        tx.instant();
    });
    rt.frame(0.0);
    renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &mut rt.text.atlas,
        &rt.instances,
        (256, 64),
    );
    assert_eq!(renderer.layer_stats().targets, 2);
    assert_eq!(renderer.layer_stats().bytes, 64 * 1024);
    assert_eq!(renderer.layer_stats().extra_passes, 2);
    let allocations = renderer.layer_stats().allocations;
    renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &mut rt.text.atlas,
        &rt.instances,
        (256, 64),
    );
    assert_eq!(renderer.layer_stats().bytes, 64 * 1024);
    assert_eq!(
        renderer.layer_stats().extra_passes,
        0,
        "both smaller caches can hit"
    );
    assert_eq!(renderer.layer_stats().allocations, allocations);
}
