use super::*;
use crate::window::gpu_failure::Policy;

/// On the GPU thread: the device (the one `warm_gpu` made, if it did), the renderer, the
/// uncommitted visual and a swap chain on it, configured for `size`.
pub(super) fn prepare(
    hwnd: isize,
    size: (u32, u32),
    srgb: bool,
    features: wgpu::Features,
    latest: &Mutex<Vec<crate::render::Instance>>,
    policy: &Policy,
) -> Result<Prepared, String> {
    policy.observe(None)?;
    let handover = Handover::new(hwnd)?;
    let visual = handover.visual();
    // SAFETY: the visual stays alive while the surface does: the surface takes its own reference
    // to it, and the window keeps the `Handover` as long as it keeps the surface.
    let (gpu, surface, renderer) = Gpu::for_owned_surface(policy, features, false, |instance| {
        unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CompositionVisual(visual))
        }
        .map_err(|e| e.to_string())
    })?;
    policy.observe(Some(&gpu))?;
    let (config, _) = surface_config(&surface, &gpu.adapter, (size.0.max(1), size.1.max(1)), srgb)
        .ok_or("the GPU can't present to this window")?;
    configure(&surface, &gpu, &config)?;
    policy.observe(Some(&gpu))?;
    let target_format = if srgb {
        config.format.remove_srgb_suffix()
    } else {
        config.format
    };
    let mut renderer = renderer.unwrap_or_else(|| Renderer::new(&gpu.device, target_format));
    let instances = std::mem::take(&mut *latest.lock().unwrap_or_else(|e| e.into_inner()));
    warm_frame(
        &gpu,
        &mut renderer,
        target_format,
        (config.width, config.height),
        &instances,
        policy,
    )?;
    policy.observe(Some(&gpu))?;
    crate::marks::mark("gpu.window.warmed");
    Ok(Prepared {
        gpu,
        renderer,
        surface,
        config,
        handover,
    })
}

/// On the GPU thread, before the hand-over: the renderer draws the window's latest CPU frame once,
/// into a target of the swap chain's format, and waits for the GPU. Its first frame's costs are
/// paid here, so the hand-over frame on the app's thread costs what a frame does: the pipelines
/// its layers' formats need, its layers' targets (the hand-over frame's layers are the latest
/// frame's, or nearly), the staging space and the instance buffer. Its own target goes as soon as
/// the GPU is done with it. The atlas is an empty one: the glyphs land with the hand-over frame's
/// own prepare.
fn warm_frame(
    gpu: &Gpu,
    renderer: &mut Renderer,
    format: wgpu::TextureFormat,
    size: (u32, u32),
    instances: &[crate::render::Instance],
    policy: &Policy,
) -> Result<(), String> {
    policy.observe(Some(gpu))?;
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rune warm frame"),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut atlas = crate::text::Atlas::new();
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    renderer.prepare_ui(&gpu.device, &gpu.queue, &mut atlas, instances, size);
    renderer.draw(&mut enc, &view, [0.0; 4]);
    if renderer
        .submit_if(&gpu.queue, enc, || !policy.failed_on(gpu))
        .is_none()
    {
        policy.observe(Some(gpu))?;
        return Err("the GPU stopped before its preparation frame was submitted".into());
    }
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    target.destroy();
    let _ = gpu.device.poll(wgpu::PollType::Poll);
    policy.observe(Some(gpu))
}

impl<S: 'static> Host<S> {
    /// The first idle turn after the window's first CPU frame: the GPU starts on its thread
    /// (`RUNE_CPU_FIRST=only`: not until a frame needs it, for checking the CPU window alone).
    pub(in crate::window) fn cpu_idle(&mut self) {
        if self.gpu_failed() {
            return;
        }
        if self.gpu_owed() {
            self.start_gpu(true);
        }
    }

    /// Whether the GPU is due to start now (`cpu_idle`).
    pub(in crate::window) fn gpu_owed(&self) -> bool {
        if std::env::var_os("RUNE_CPU_FIRST").is_some_and(|v| v == "only") {
            return false;
        }
        let now = Instant::now();
        self.win.as_ref().is_some_and(|w| matches!(w.native.mode, SurfaceMode::Cpu { job: GpuJob::Waiting { retry_at, .. }, .. } if retry_at.is_none_or(|at| at <= now)) && w.first_frame.ready())
    }

    /// Starts making the GPU for this window on a thread (`below`: at below-normal priority, as
    /// after the first frame; a frame that needs the GPU makes it at normal priority).
    pub(super) fn start_gpu(&mut self, below: bool) {
        if self.gpu_failed() {
            return;
        }
        let Some(w) = self.win.as_mut() else { return };
        let SurfaceMode::Cpu { job, .. } = &mut w.native.mode else {
            return;
        };
        let GpuJob::Waiting { tries, retry_at } = *job else {
            return;
        };
        if retry_at.is_some() {
            crate::marks::mark("gpu.window.retry");
        }
        let Some(hwnd) = crate::platform::win32::hwnd(&w.window) else {
            *job = GpuJob::Failed;
            return;
        };
        let slot = Arc::new(Mutex::new(None));
        let (out, handle, srgb, size) = (
            slot.clone(),
            self.native_handle.clone(),
            self.rt.srgb_blend(),
            (w.config.width, w.config.height),
        );
        // What the window draws, for the GPU thread's first frame of its own (`warm_frame`): each CPU
        // frame until then replaces it.
        let latest = Arc::new(Mutex::new(warm_instances(&self.rt.instances)));
        let instances = latest.clone();
        let features = self
            .comp
            .as_ref()
            .map_or(wgpu::Features::empty(), |c| c.features());
        let hwnd = hwnd as isize;
        let policy = self.hooks.gpu_failure.clone();
        let spawned = std::thread::Builder::new()
            .name("rune-gpu-window".into())
            .spawn(move || {
                if below {
                    lower_priority();
                }
                let made = prepare(hwnd, size, srgb, features, &instances, &policy);
                if let Err(reason) = &made {
                    policy.record(reason);
                }
                *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(made);
                handle.send(WakeEvent::GpuReady);
            });
        match spawned {
            Ok(thread) => {
                crate::marks::mark("gpu.window.start");
                *job = GpuJob::Building {
                    slot,
                    latest,
                    thread,
                    since: Instant::now(),
                    raised: !below,
                    tries: tries + 1,
                    animating: false,
                    ready: None,
                };
            }
            Err(error) => {
                self.cpu_gpu_failed(format!("its thread couldn't start: {error}"), tries + 1)
            }
        }
    }

    /// The GPU thread finished: attach its device, renderer and surface and start handing over,
    /// or stay on the CPU if it failed. While handing over: the GPU finished a frame.
    pub(in crate::window) fn gpu_ready(&mut self) {
        if self.gpu_failed() {
            return;
        }
        if self
            .win
            .as_ref()
            .is_some_and(|w| matches!(w.native.mode, SurfaceMode::HandingOver { .. }))
        {
            return self.hand_over_if_done();
        }
        let Some(w) = self.win.as_mut() else { return };
        let SurfaceMode::Cpu {
            job:
                GpuJob::Building {
                    slot,
                    tries,
                    animating,
                    ready,
                    ..
                },
            ..
        } = &mut w.native.mode
        else {
            return;
        };
        let tries = *tries;
        let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(Ok(prepared)) = slot.as_ref()
            && let Err(reason) = self
                .hooks
                .gpu_failure
                .watch(&prepared.gpu, || setup_waker(&self.shared))
        {
            drop(slot);
            self.fail_gpu(reason);
            return;
        }
        // Ready while the CPU frames move something (a view fading in): the hand-over waits for a
        // frame at rest, so the frame both renderers draw is one that costs what a frame at rest
        // does, not the first of the GPU's frames to draw every layer of an entrance.
        let now = Instant::now();
        if slot.as_ref().is_some_and(Result::is_ok)
            && *animating
            && now < *ready.get_or_insert(now) + QUIET_WAIT
        {
            crate::marks::mark("gpu.window.deferred");
            return;
        }
        let Some(made) = slot.take() else { return };
        drop(slot);
        let prepared = match made {
            Ok(prepared) => prepared,
            Err(error) => return self.cpu_gpu_failed(error, tries),
        };
        crate::marks::mark("gpu.window.ready");
        let Prepared {
            gpu,
            mut renderer,
            surface,
            config,
            handover,
        } = prepared;
        let gpu = Arc::new(gpu);
        let target_format = if self.rt.srgb_blend() {
            config.format.remove_srgb_suffix()
        } else {
            config.format
        };
        rebuild_renderer(&gpu.device, &mut renderer, target_format, &mut self.rt);
        if let Err(reason) = self.hooks.gpu_failure.observe(Some(&gpu)) {
            self.fail_gpu(reason);
            return;
        }
        *self.gpu_pool.borrow_mut() = Arc::downgrade(&gpu);
        crate::render::surface_gpu(&gpu);
        let size = w.window.inner_size();
        let SurfaceMode::Cpu { mut cpu, .. } =
            std::mem::replace(&mut w.native.mode, SurfaceMode::Gpu)
        else {
            unreachable!()
        };
        cpu.half = HalfRounding::of_vendor(gpu.adapter.get_info().vendor);
        w.native
            .video_layers
            .hold_premultiplied(vrr_guard(gpu.adapter.get_info().vendor));
        w.native.mode = SurfaceMode::HandingOver {
            cpu,
            handover,
            done: None,
            readback: None,
            since: Instant::now(),
            nudged: None,
            tries,
            paused: false,
        };
        w.native.slot.reset();
        w.native.damage.reset();
        // D3D12's idle-priority threads compile the first frame's shaders: on a busy machine they
        // would hold the hand-over frame off.
        crate::platform::win32::unidle_d3d_threads_soon();
        w.surface = Some(surface);
        w.config = config;
        w.surface_ok = true;
        // The window changed size while the GPU was made.
        if size.width > 0 && size.height > 0 {
            w.resize_surface(Some(&*gpu), size);
        }
        self.gpu = Some((gpu, renderer));
        w.request_redraw();
    }

    /// While handing over: the GPU draws the instances the CPU just painted, into its swap chain
    /// on the uncommitted visual.
    pub(super) fn gpu_draws_same_frame(&mut self) {
        if self.gpu_failed() {
            return;
        }
        let (Some(l), Some((gpu, renderer))) = (self.win.as_mut(), self.gpu.as_mut()) else {
            return;
        };
        let SurfaceMode::HandingOver {
            done,
            readback,
            paused: false,
            ..
        } = &mut l.native.mode
        else {
            return;
        };
        let Some(surface) = l.surface.as_ref() else {
            return;
        };
        let Ok(frame) = acquire(surface, gpu, &l.config, l.surface_ok, 1, &mut self.meter) else {
            return;
        };
        if self.hooks.gpu_failure.failed_on(gpu) {
            self.discard_failed_frame(frame);
            return;
        }
        let rt = &mut self.rt;
        let target_format = if rt.srgb_blend() {
            l.config.format.remove_srgb_suffix()
        } else {
            l.config.format
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: (target_format != l.config.format).then_some(target_format),
            ..Default::default()
        });
        renderer.upload_images(&gpu.device, &gpu.queue, &mut rt.images);
        renderer.prepare_surfaces(
            &gpu.device,
            &gpu.queue,
            &rt.surfaces,
            (l.config.width, l.config.height),
        );
        if self.hooks.gpu_failure.failed_on(gpu) {
            self.discard_failed_frame(frame);
            return;
        }
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        // The app's compositor draws the same instances the CPU painted (the CPU frame drew none of
        // its effect slots, or it would have asked for the GPU).
        renderer.set_frame_wake(rt.live_wake());
        if let Some(c) = self.comp.as_mut() {
            let size = (l.config.width, l.config.height);
            c.render(&mut Frame {
                device: &gpu.device,
                queue: &gpu.queue,
                encoder: &mut enc,
                target: &view,
                format: target_format,
                size,
                dt: 0.0,
                update_ms: 0.0,
                rt: &mut *rt,
                ui: &mut *renderer,
            });
        } else {
            renderer.prepare_ui(
                &gpu.device,
                &gpu.queue,
                &mut rt.text.atlas,
                &rt.instances,
                (l.config.width, l.config.height),
            );
            renderer.draw(&mut enc, &view, rt.clear_color());
        }
        if self.hooks.gpu_failure.failed_on(gpu) {
            self.discard_failed_frame(frame);
            return;
        }
        // A copy of the frame, for the drift count once it's the one the window commits to: only
        // when it's asked for (`RUNE_CPU_BOOT_DRIFT`), as the count is a diagnostic.
        *readback = (crate::render::cpu_boot::drift_wanted()
            && l.config.usage.contains(wgpu::TextureUsages::COPY_SRC))
        .then(|| {
            let (width, height) = (l.config.width, l.config.height);
            let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rune hand-over readback"),
                size: u64::from(row) * u64::from(height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            enc.copy_texture_to_buffer(
                frame.texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            Readback {
                buffer,
                row,
                size: (width, height),
            }
        });
        if renderer
            .submit_if(&gpu.queue, enc, || !self.hooks.gpu_failure.failed_on(gpu))
            .is_none()
        {
            self.discard_failed_frame(frame);
            return;
        }
        if self.hooks.gpu_failure.failed_on(gpu) {
            self.discard_failed_frame(frame);
            return;
        }
        gpu.queue.present(frame);
        if self.hooks.gpu_failure.failed_on(gpu) {
            self.gpu_failed();
            return;
        }
        l.native.slot.presented();
        let flag = Arc::new(AtomicBool::new(false));
        let set = flag.clone();
        gpu.queue
            .on_submitted_work_done(move || set.store(true, Ordering::Release));
        *done = Some((flag, Instant::now()));
        // A thread waits for the GPU and wakes the loop, which commits (`hand_over_if_done`).
        let (waiting, handle) = (gpu.clone(), self.native_handle.clone());
        let spawned = std::thread::Builder::new()
            .name("rune-gpu-wait".into())
            .spawn(move || {
                let _ = waiting.device.poll(wgpu::PollType::wait_indefinitely());
                handle.send(WakeEvent::GpuReady);
            });
        if let Err(error) = spawned {
            eprintln!("rune: could not wait for the GPU's frame: {error}");
        }
        crate::marks::mark_once(&FIRST_GPU, "frame.first.gpu");
        // A first frame only the GPU could draw.
        if !l.first_frame.ready() {
            l.first_frame.gpu_done();
        }
    }

    /// At the start of a turn while handing over: when the GPU's last frame is done, commit the
    /// visual (DWM shows the GPU from its next composition) and leave the CPU behind; the CPU's
    /// buffer goes on a worker after counting the frame's drift.
    pub(in crate::window) fn hand_over_if_done(&mut self) {
        if self.gpu_failed() {
            return;
        }
        let (Some(w), Some((gpu, _))) = (self.win.as_mut(), self.gpu.as_ref()) else {
            return;
        };
        match &mut w.native.mode {
            SurfaceMode::HandingOver {
                done: Some((done, _)),
                tries,
                ..
            } => {
                let tries = *tries;
                let _ = gpu.device.poll(wgpu::PollType::Poll);
                if let Some(reason) = self.hooks.gpu_failure.reason(Some(gpu)) {
                    return self.cpu_gpu_failed(reason, tries);
                }
                let lost = gpu.is_lost();
                if !lost && !done.load(Ordering::Acquire) {
                    return;
                }
                let committed = if lost {
                    Err("its device was lost".to_owned())
                } else if let SurfaceMode::HandingOver { handover, .. } = &w.native.mode {
                    handover.commit()
                } else {
                    unreachable!()
                };
                if let Err(error) = committed {
                    return self.cpu_gpu_failed(
                        format!("the GPU couldn't take over the window: {error}"),
                        tries,
                    );
                }
                let SurfaceMode::HandingOver {
                    cpu,
                    handover,
                    readback,
                    ..
                } = std::mem::replace(&mut w.native.mode, SurfaceMode::Gpu)
                else {
                    unreachable!()
                };
                crate::marks::mark("gpu.window.handed");
                crate::platform::win32::unidle_d3d_threads_soon();
                // A first frame only the GPU could draw showed cloaked: it comes on screen now.
                if w.native.cloak.held() {
                    w.native.cloak.lift(&w.window);
                    crate::marks::mark("window.shown");
                }
                // What DWM shows is in the window's own surface now, not in the buffer: it goes on a
                // worker, after counting how many of its pixels the GPU's same frame drew otherwise.
                // A frame only the GPU draws (Slug's text, a native surface) never showed a CPU
                // frame: its buffer holds nothing to count against.
                let shown = matches!(cpu.shown, Shown::Copied(_));
                if let Some(dib) = cpu.into_dib() {
                    match readback {
                        Some(r) if shown => count_drift(gpu.clone(), r, dib),
                        Some(_) => {
                            eprintln!(
                                "rune: cpu boot drift: none to count, the CPU showed no frame (the first needed the GPU)"
                            );
                            free_later(dib);
                        }
                        None => free_later(dib),
                    }
                }
                w.native.mode = SurfaceMode::Handed { handover };
                w.presented = None;
                // What was asked for while the GPU finished its frame draws now, on the GPU.
                w.request_redraw();
            }
            SurfaceMode::HandingOver { done: None, .. }
            | SurfaceMode::Gpu
            | SurfaceMode::Cpu { .. }
            | SurfaceMode::Handed { .. }
            | SurfaceMode::Rebinding { .. } => {}
        }
    }

    pub(in crate::window) fn pending_gpu_failure(&self) -> Option<String> {
        let SurfaceMode::Cpu {
            job: GpuJob::Building { slot, .. },
            ..
        } = &self.win.as_ref()?.native.mode
        else {
            return None;
        };
        slot.lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|made| match made {
                Ok(prepared) => self.hooks.gpu_failure.reason(Some(&prepared.gpu)),
                Err(reason) => Some(reason.clone()),
            })
    }

    /// The GPU couldn't take over (`why`), on its `tries`th try: the window goes on, or back, to
    /// the CPU renderer, and tries again after `RETRY_AFTER`, or stays there after `TRIES` tries.
    fn cpu_gpu_failed(&mut self, why: String, tries: u8) {
        if !matches!(self.hooks.gpu_failure, super::gpu_failure::Policy::Recover) {
            self.fail_gpu(why);
            return;
        }
        eprintln!(
            "rune: the window stays on the CPU renderer for now (try {tries} of {TRIES}): {why}"
        );
        crate::marks::mark("gpu.window.failed");
        if !crate::marks::noted("gpu.window.why") {
            crate::marks::note("gpu.window.why", Box::leak(why.into_boxed_str()));
        }
        let Some(w) = self.win.as_mut() else { return };
        let job = if tries < TRIES {
            GpuJob::Waiting {
                tries,
                retry_at: Some(Instant::now() + RETRY_AFTER),
            }
        } else {
            GpuJob::Failed
        };
        let (SurfaceMode::Cpu { cpu, .. } | SurfaceMode::HandingOver { cpu, .. }) =
            std::mem::replace(&mut w.native.mode, SurfaceMode::Gpu)
        else {
            unreachable!("only a CPU window hands over")
        };
        w.native.mode = SurfaceMode::Cpu { cpu, job };
        // From a hand-over: its swap chain and GPU go. DWM kept the window's own content, which
        // the CPU goes on drawing.
        w.surface = None;
        self.gpu = None;
        w.request_redraw();
    }

    /// The hand-over's deadlines, at the start of a turn (`Win::handover_deadline` wakes the loop
    /// for them): a GPU thread still making the GPU is raised to normal priority, then logged as
    /// slow; a frame both renderers draw is asked for until one is, in sight or not; a GPU that
    /// doesn't finish that frame fails; a failed try is made again.
    pub(in crate::window) fn watch_handover(&mut self, now: Instant) {
        if self.gpu_failed() {
            return;
        }
        let Some(w) = self.win.as_mut() else { return };
        match &mut w.native.mode {
            SurfaceMode::Cpu {
                job:
                    GpuJob::Building {
                        thread,
                        since,
                        raised,
                        ready,
                        ..
                    },
                ..
            } => {
                if ready.is_some_and(|at| now >= at + QUIET_WAIT) {
                    return self.gpu_ready();
                }
                if !*raised && now >= *since + RAISE_AFTER {
                    *raised = true;
                    raise_priority(thread);
                    crate::marks::mark("gpu.window.raised");
                }
                if now >= *since + BUILD_SLOW {
                    crate::marks::mark("gpu.window.slow");
                }
            }
            SurfaceMode::HandingOver {
                done: None,
                since,
                nudged,
                tries,
                ..
            } => {
                if now >= *since + FINISH_LIMIT {
                    let tries = *tries;
                    return self.cpu_gpu_failed(
                        format!(
                            "no frame could be drawn by both renderers in {} s",
                            FINISH_LIMIT.as_secs()
                        ),
                        tries,
                    );
                }
                if now < nudged.unwrap_or(*since) + NUDGE_EVERY {
                    return;
                }
                *nudged = Some(now);
                crate::marks::mark("gpu.window.nudged");
                w.request_redraw();
            }
            SurfaceMode::HandingOver {
                done: Some((_, at)),
                tries,
                ..
            } => {
                if now < *at + FINISH_LIMIT {
                    return;
                }
                let tries = *tries;
                self.cpu_gpu_failed(
                    format!(
                        "the GPU didn't finish the hand-over frame in {} s",
                        FINISH_LIMIT.as_secs()
                    ),
                    tries,
                );
            }
            SurfaceMode::Cpu {
                job: GpuJob::Waiting {
                    retry_at: Some(at), ..
                },
                ..
            } => {
                if now >= *at && w.first_frame.ready() {
                    self.start_gpu(true);
                }
            }
            SurfaceMode::Cpu {
                job: GpuJob::Waiting { retry_at: None, .. } | GpuJob::Failed,
                ..
            }
            | SurfaceMode::Gpu
            | SurfaceMode::Handed { .. }
            | SurfaceMode::Rebinding { .. } => {}
        }
    }
}
