use super::*;

impl SurfaceRenderer {
    pub fn submit(
        &self,
        queue: &wgpu::Queue,
        encoder: wgpu::CommandEncoder,
    ) -> wgpu::SubmissionIndex {
        self.submit_if(queue, encoder, || true).unwrap()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn submit_if(
        &self,
        queue: &wgpu::Queue,
        encoder: wgpu::CommandEncoder,
        allowed: impl Fn() -> bool,
    ) -> Option<wgpu::SubmissionIndex> {
        assert!(
            self.draws.is_empty(),
            "native video surfaces are unsupported on web"
        );
        let commands = encoder.finish();
        allowed().then(|| queue.submit([commands]))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn submit_if(
        &self,
        queue: &wgpu::Queue,
        encoder: wgpu::CommandEncoder,
        allowed: impl Fn() -> bool,
    ) -> Option<wgpu::SubmissionIndex> {
        let submitted_at = Instant::now();
        let frames: Vec<_> = self
            .draws
            .iter()
            .filter(|p| p.drawn.load(std::sync::atomic::Ordering::Relaxed))
            .map(|p| {
                (
                    p.surface.clone(),
                    p.imported.clone(),
                    FrameTiming {
                        frame_id: p.imported.frame.id,
                        submitted_at,
                        completed_at: None,
                        gpu_start_seconds: None,
                        gpu_end_seconds: None,
                    },
                )
            })
            .collect();
        if frames.is_empty() {
            let commands = encoder.finish();
            return allowed().then(|| queue.submit([commands]));
        }
        let reports: Vec<_> = frames
            .iter()
            .map(|(surface, _, timing)| (surface.clone(), timing.clone()))
            .collect();
        // A caller may use Renderer::draw_timed with its own query set. Never resolve our
        // unwritten counters in that case; CPU submission/completion notifications still work.
        if let Some(queries) = self
            .queries
            .as_ref()
            .filter(|_| self.queries_used.load(std::sync::atomic::Ordering::Relaxed))
        {
            let resolve = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("video timestamps"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("video timestamp readback"),
                size: 16,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let seconds_per_tick = queue.get_timestamp_period() as f64 * 1e-9;
            let queries = queries.clone();
            let device = self.device.clone();
            let queue = queue.clone();
            let completion = self.completion.clone();
            // Metal counter resolves can race the render pass's final fragment sample when
            // encoded into the same command buffer. Resolve after its completion fence; this
            // is timestamp metadata only, never a readback or wait on the video pixel path.
            encoder.on_submitted_work_done(move || {
                let completed_at = Instant::now();
                let mut timestamps = device.create_command_encoder(&Default::default());
                timestamps.resolve_query_set(
                    &queries.pool.queries,
                    queries.start..queries.start + 2,
                    &resolve,
                    0,
                );
                timestamps.copy_buffer_to_buffer(&resolve, 0, &readback, 0, 16);
                let result = readback.clone();
                timestamps.map_buffer_on_submit(
                    &readback,
                    wgpu::MapMode::Read,
                    ..,
                    move |status| {
                        let times = if status.is_ok() {
                            let bytes = result
                                .slice(..)
                                .get_mapped_range()
                                .expect("mapped timestamps");
                            let a = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                            let b = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                            (b >= a && b != 0).then_some((
                                a as f64 * seconds_per_tick,
                                b as f64 * seconds_per_tick,
                            ))
                        } else {
                            None
                        };
                        if status.is_ok() {
                            result.unmap();
                        }
                        // The sample pair must survive both GPU submissions and CPU mapping.
                        drop(queries);
                        for (s, _, mut t) in frames {
                            t.completed_at = Some(completed_at);
                            if let Some((a, b)) = times {
                                t.gpu_start_seconds = Some(a);
                                t.gpu_end_seconds = Some(b);
                            }
                            s.report(t);
                        }
                    },
                );
                queue.submit([timestamps.finish()]);
                let _ = completion.try_send(());
            });
        } else {
            encoder.on_submitted_work_done(move || {
                for (s, _, mut t) in frames {
                    t.completed_at = Some(Instant::now());
                    s.report(t);
                }
            });
        }
        let commands = encoder.finish();
        if !allowed() {
            return None;
        }
        // Notify before submission: completion may race queue.submit returning.
        for (surface, timing) in reports {
            surface.report(timing);
        }
        if !allowed() {
            return None;
        }
        let index = queue.submit([commands]);
        self.release_decoded(queue);
        let _ = self.completion.try_send(());
        Some(index)
    }
}
