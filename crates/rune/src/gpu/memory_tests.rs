use super::hints;

#[test]
fn allocation_policy_is_limited_to_linux_vulkan() {
    for backend in [
        wgpu::Backend::Noop,
        wgpu::Backend::Vulkan,
        wgpu::Backend::Metal,
        wgpu::Backend::Dx12,
        wgpu::Backend::Gl,
        wgpu::Backend::BrowserWebGpu,
    ] {
        match hints(backend) {
            wgpu::MemoryHints::Manual {
                suballocated_device_memory_block_size: device,
            } => {
                assert!(cfg!(target_os = "linux"));
                assert_eq!(backend, wgpu::Backend::Vulkan);
                assert_eq!(device, (16 << 20)..(64 << 20));
                // The pinned HAL derives host blocks at half the device sizes.
                assert_eq!((device.start / 2)..(device.end / 2), (8 << 20)..(32 << 20));
            }
            wgpu::MemoryHints::MemoryUsage => {
                assert!(!cfg!(target_os = "linux") || backend != wgpu::Backend::Vulkan);
            }
            _ => panic!("unexpected allocation policy for {backend:?}"),
        }
    }
}
