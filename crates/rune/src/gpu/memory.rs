//! Native allocation policy; block sizes do not cap total memory.

pub(crate) fn hints(backend: wgpu::Backend) -> wgpu::MemoryHints {
    if cfg!(target_os = "linux") && backend == wgpu::Backend::Vulkan {
        // Only Linux/Intel was measured: doubling the initial blocks absorbs this pane workload's
        // first-use churn. It moves the next boundary; larger workloads can still reach it.
        // Keep the old 64/32 MiB growth ceilings; small-app startup overhead is unmeasured.
        wgpu::MemoryHints::Manual {
            suballocated_device_memory_block_size: (16 << 20)..(64 << 20),
        }
    } else {
        // Performance can reserve about 190 MiB extra on integrated GPUs and on Windows;
        // keep MemoryUsage on backends whose startup footprint was not measured here.
        wgpu::MemoryHints::MemoryUsage
    }
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
