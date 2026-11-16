use super::*;

#[test]
fn measured_framebuffer_composites_keep_raw_differences() {
    let mut gpu = GpuRenderer::default();
    let scenes = [
        serde_json::from_str(include_str!("fixtures/intel-minimized.json")).unwrap(),
        serde_json::from_str(include_str!("fixtures/lvp-minimized.json")).unwrap(),
        generate::radial(57, 8),
        generate::seeded(93, 8),
        generate::seeded(4, 8),
        effect_slots(generate::seeded(4, 8)),
    ];
    for scene in scenes {
        let mut cpu = Frame::new(scene.width, scene.height, true);
        cpu.render(&scene);
        let actual = gpu.render(&scene);
        let raw = compare(&scene, &cpu, &actual, 2);
        let qualified = check(&mut gpu, &scene, 2).expect("framebuffer composite qualified");
        assert_eq!(qualified, raw.max_channel);
    }
}
