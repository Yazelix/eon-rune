use super::*;

#[test]
fn actual_uncaptured_validation_is_recorded_by_the_surface_handler() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::NOOP;
    descriptor.backend_options.noop.enable = true;
    let instance = wgpu::Instance::new(descriptor);
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let gpu = Gpu::from_device(instance, adapter, device, queue);
    log_uncaptured(&gpu.device, gpu.failure.clone());
    let _invalid = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4,
        usage: wgpu::BufferUsages::empty(),
        mapped_at_creation: false,
    });
    assert!(gpu.failure.message().unwrap().contains("Validation Error"));
    assert!(
        !gpu.is_lost(),
        "validation is fatal only for apps selecting terminal handling"
    );
}
