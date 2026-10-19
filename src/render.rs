use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, RecvTimeoutError};
use windows::{
    Win32::{
        Foundation::{E_FAIL, LUID},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D11::*,
            DirectComposition::*,
            Dxgi::{Common::*, *},
        },
    },
    core::{Error, Interface, Result},
};

use crate::{
    color::{ColorState, ColorTransform},
    gpu::{Gpu, Texture},
    window::{Overlay, WindowClass},
};

#[derive(Default)]
pub struct FilterControl {
    pub enabled: AtomicBool,
    pub frozen: AtomicBool,
}

pub fn render_loop(
    fragment: &'static str,
    refresh_rate: u32,
    terminator: Receiver<()>,
    control: Arc<FilterControl>,
) -> Result<()> {
    let class = WindowClass::new()?;
    let mut monitors = Vec::<MonitorSlot>::new();
    let mut next_scan = Instant::now();
    let mut next_frame = Instant::now();
    let interval = if refresh_rate == 0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(1.0 / refresh_rate as f64)
    };

    loop {
        if !class.process_messages()
            || monitors.iter().any(|slot| {
                slot.monitor
                    .as_ref()
                    .is_some_and(|monitor| monitor.overlay.closed())
            })
        {
            return Ok(());
        }
        if monitors.iter().any(|slot| {
            slot.monitor
                .as_ref()
                .is_some_and(|monitor| monitor.overlay.changed())
        }) {
            next_scan = Instant::now();
        }
        let now = Instant::now();
        if now >= next_scan {
            let snapshot = unsafe { CreateDXGIFactory1::<IDXGIFactory2>() }
                .and_then(|factory| MonitorSpec::enumerate(&factory).map(|specs| (factory, specs)));
            match snapshot {
                Ok((factory, specs)) => {
                    let mut previous = std::mem::take(&mut monitors);
                    for spec in specs {
                        let mut slot = previous
                            .iter()
                            .position(|slot| slot.spec.same_display(&spec))
                            .map(|index| previous.swap_remove(index))
                            .unwrap_or_else(|| MonitorSlot::new(spec.clone()));
                        slot.refresh(spec, &factory, &class, fragment);
                        monitors.push(slot);
                    }
                }
                Err(error) => {
                    for slot in &mut monitors {
                        slot.pause(&error);
                    }
                    log::error!("Display enumeration failed: {error}");
                }
            }
            next_scan = now + Duration::from_secs(1);
        }

        let enabled = control.enabled.load(Ordering::Relaxed);
        let frozen = control.frozen.load(Ordering::Relaxed);
        let draw = now >= next_frame;
        for slot in &mut monitors {
            if let Some(monitor) = &mut slot.monitor
                && let Err(error) = monitor.tick(enabled, frozen, draw)
            {
                slot.pause(&error);
            } else if slot.monitor.as_ref().is_some_and(|monitor| monitor.ready) {
                slot.last_error = None;
            }
        }
        if draw {
            next_frame = now + interval;
        }

        let wait = if enabled && !frozen {
            Duration::from_millis(1)
        } else {
            Duration::from_millis(20)
        };
        match terminator.recv_timeout(wait) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

#[derive(Clone)]
struct MonitorSpec {
    adapter: IDXGIAdapter1,
    output: IDXGIOutput,
    adapter_id: LUID,
    desc: DXGI_OUTPUT_DESC,
}

impl MonitorSpec {
    fn enumerate(factory: &IDXGIFactory2) -> Result<Vec<Self>> {
        let mut specs = Vec::new();
        unsafe {
            let mut adapter_index = 0;
            loop {
                let adapter = match factory.EnumAdapters1(adapter_index) {
                    Ok(adapter) => adapter,
                    Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(error) => return Err(error),
                };
                let adapter_id = adapter.GetDesc1()?.AdapterLuid;
                let mut output_index = 0;
                loop {
                    let output = match adapter.EnumOutputs(output_index) {
                        Ok(output) => output,
                        Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                        Err(error) => return Err(error),
                    };
                    let desc = output.GetDesc()?;
                    if desc.AttachedToDesktop.as_bool() {
                        specs.push(Self {
                            adapter: adapter.clone(),
                            output,
                            adapter_id,
                            desc,
                        });
                    }
                    output_index += 1;
                }
                adapter_index += 1;
            }
        }
        Ok(specs)
    }

    fn same_display(&self, other: &Self) -> bool {
        self.adapter_id == other.adapter_id && self.desc.DeviceName == other.desc.DeviceName
    }

    fn same_layout(&self, other: &Self) -> bool {
        self.same_display(other)
            && self.desc.DesktopCoordinates == other.desc.DesktopCoordinates
            && self.desc.Rotation == other.desc.Rotation
    }

    fn name(&self) -> String {
        let end = self
            .desc
            .DeviceName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(self.desc.DeviceName.len());
        String::from_utf16_lossy(&self.desc.DeviceName[..end])
    }

    fn dimensions(&self) -> Result<(u32, u32)> {
        let rect = self.desc.DesktopCoordinates;
        let width = u32::try_from(rect.right - rect.left)
            .map_err(|_| Error::new(E_FAIL, "Invalid display width"))?;
        let height = u32::try_from(rect.bottom - rect.top)
            .map_err(|_| Error::new(E_FAIL, "Invalid display height"))?;
        if width == 0 || height == 0 {
            return Err(Error::new(E_FAIL, "Display dimensions must be positive"));
        }
        Ok((width, height))
    }
}

struct MonitorSlot {
    spec: MonitorSpec,
    monitor: Option<Monitor>,
    last_error: Option<String>,
}

impl MonitorSlot {
    fn new(spec: MonitorSpec) -> Self {
        Self {
            spec,
            monitor: None,
            last_error: None,
        }
    }

    fn refresh(
        &mut self,
        spec: MonitorSpec,
        factory: &IDXGIFactory2,
        class: &WindowClass,
        fragment: &str,
    ) {
        let color = match ColorState::read(spec.adapter_id, &spec.desc.DeviceName) {
            Ok(color) => color,
            Err(error) => {
                self.spec = spec;
                self.pause(&error);
                return;
            }
        };
        if self.spec.same_layout(&spec)
            && self
                .monitor
                .as_ref()
                .is_some_and(|monitor| monitor.color == color)
        {
            self.spec = spec;
            return;
        }
        self.monitor.take();
        self.spec = spec;
        match Monitor::new(&self.spec, color, factory, class, fragment) {
            Ok(monitor) => {
                log::info!("Display {} resources created", self.spec.name());
                self.monitor = Some(monitor);
            }
            Err(error) => self.pause(&error),
        }
    }

    fn pause(&mut self, error: &Error) {
        self.monitor.take();
        let message = format!("{}: {error}", self.spec.name());
        if self.last_error.as_ref() != Some(&message) {
            log::error!("Display paused: {message}");
            crate::notify("Screen Filter display paused", &message);
            self.last_error = Some(message);
        }
    }
}

struct Monitor {
    gpu: Gpu,
    duplication: IDXGIOutputDuplication,
    swap_chain: IDXGISwapChain3,
    _composition: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    transform: Option<ColorTransform>,
    frames: Option<FrameTextures>,
    color: ColorState,
    adapter_id: LUID,
    device_name: [u16; 32],
    rotation: DXGI_MODE_ROTATION,
    width: u32,
    height: u32,
    ready: bool,
    frozen: bool,
    overlay: Overlay,
}

impl Monitor {
    fn new(
        spec: &MonitorSpec,
        color: ColorState,
        factory: &IDXGIFactory2,
        class: &WindowClass,
        fragment: &str,
    ) -> Result<Self> {
        let (width, height) = spec.dimensions()?;
        let overlay = class
            .create(spec.desc.DesktopCoordinates)
            .map_err(|error| Error::new(error.code(), format!("Overlay creation: {error}")))?;
        unsafe {
            let adapter = spec.adapter.cast::<IDXGIAdapter>()?;
            let gpu = Gpu::new(Some(&adapter), D3D_DRIVER_TYPE_UNKNOWN, fragment)?;
            let dxgi_device = gpu.device.cast::<IDXGIDevice>()?;
            let transform = match &color {
                ColorState::Advanced => None,
                ColorState::Legacy { profile, .. } => {
                    Some(ColorTransform::new(&dxgi_device, profile).map_err(|error| {
                        Error::new(error.code(), format!("ICC transform creation: {error}"))
                    })?)
                }
            };
            let duplication = spec
                .output
                .cast::<IDXGIOutput5>()?
                .DuplicateOutput1(
                    &gpu.device,
                    0,
                    &[
                        DXGI_FORMAT_R16G16B16A16_FLOAT,
                        DXGI_FORMAT_B8G8R8A8_UNORM,
                        DXGI_FORMAT_R8G8B8A8_UNORM,
                    ],
                )
                .map_err(|error| {
                    Error::new(
                        error.code(),
                        format!("Desktop duplication creation: {error}"),
                    )
                })?;
            let swap_chain = factory
                .CreateSwapChainForComposition(
                    &gpu.device,
                    &DXGI_SWAP_CHAIN_DESC1 {
                        Width: width,
                        Height: height,
                        Format: color.output_format(),
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                        BufferCount: 2,
                        Scaling: DXGI_SCALING_STRETCH,
                        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                        ..Default::default()
                    },
                    None,
                )
                .map_err(|error| {
                    Error::new(
                        error.code(),
                        format!("Composition swap chain creation: {error}"),
                    )
                })?
                .cast::<IDXGISwapChain3>()?;
            if matches!(color, ColorState::Advanced) {
                let space = DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709;
                if swap_chain.CheckColorSpaceSupport(space)?
                    & DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT.0 as u32
                    == 0
                {
                    return Err(Error::new(E_FAIL, "scRGB presentation is required"));
                }
                swap_chain.SetColorSpace1(space)?;
            }
            let composition = DCompositionCreateDevice::<_, IDCompositionDevice>(&dxgi_device)?;
            let target = composition
                .CreateTargetForHwnd(overlay.handle, true)
                .map_err(|error| {
                    Error::new(error.code(), format!("Composition window target: {error}"))
                })?;
            let visual = composition.CreateVisual()?;
            visual.SetContent(&swap_chain)?;
            target.SetRoot(&visual)?;
            composition.Commit()?;
            Ok(Self {
                gpu,
                duplication,
                swap_chain,
                _composition: composition,
                _target: target,
                _visual: visual,
                transform,
                frames: None,
                color,
                adapter_id: spec.adapter_id,
                device_name: spec.desc.DeviceName,
                rotation: spec.desc.Rotation,
                width,
                height,
                ready: false,
                frozen: false,
                overlay,
            })
        }
    }

    fn tick(&mut self, enabled: bool, frozen: bool, draw: bool) -> Result<()> {
        if self.frozen && !frozen {
            self.overlay.set_frozen(false)?;
            self.frozen = false;
        }
        if enabled && draw && (!frozen || !self.ready) {
            self.capture()?;
        }
        if frozen && self.ready && !self.frozen {
            self.overlay.set_frozen(true)?;
            self.frozen = true;
        }
        self.overlay.show(enabled && self.ready);
        Ok(())
    }

    fn capture(&mut self) -> Result<()> {
        let Some(frame) = AcquiredFrame::next(&self.duplication)? else {
            return Ok(());
        };
        self.validate_color_state()?;
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            frame.texture.GetDesc(&mut desc);
            self.color.validate_format(desc.Format)?;
            let rotated = matches!(
                self.rotation,
                DXGI_MODE_ROTATION_ROTATE90 | DXGI_MODE_ROTATION_ROTATE270
            );
            let expected = if rotated {
                (self.height, self.width)
            } else {
                (self.width, self.height)
            };
            if (desc.Width, desc.Height) != expected {
                return Err(Error::new(
                    E_FAIL,
                    "Capture dimensions changed, rebuilding display resources",
                ));
            }
            if self
                .frames
                .as_ref()
                .is_some_and(|frames| frames.format != desc.Format)
            {
                return Err(Error::new(
                    E_FAIL,
                    "Capture format changed, rebuilding display resources",
                ));
            }
            if self.frames.is_none() {
                self.frames = Some(FrameTextures::new(
                    &self.gpu,
                    &self.color,
                    &desc,
                    self.width,
                    self.height,
                )?);
            }
            let frames = self.frames.as_ref().expect("Frame textures initialized");
            self.gpu
                .context
                .CopyResource(&frames.input.raw, &frame.texture);
            frame.release()?;
            let back_buffer = self.swap_chain.GetBuffer::<ID3D11Texture2D>(0)?;
            if let Some(transform) = &self.transform {
                let (linear, gray) = frames
                    .legacy
                    .as_ref()
                    .expect("Legacy color transforms have linear frame textures");
                transform.decode(&frames.input.raw, &linear.raw)?;
                self.gpu.draw(linear, &gray.raw, self.rotation)?;
                transform.encode(&gray.raw, &back_buffer)?;
            } else {
                self.gpu.draw(&frames.input, &back_buffer, self.rotation)?;
            }
            self.validate_color_state()?;
            self.swap_chain.Present(0, DXGI_PRESENT::default()).ok()?;
            self.gpu.device.GetDeviceRemovedReason()?;
            self.ready = true;
        }
        Ok(())
    }
}

impl Monitor {
    fn validate_color_state(&self) -> Result<()> {
        if ColorState::read(self.adapter_id, &self.device_name)? != self.color {
            return Err(Error::new(
                E_FAIL,
                "Display color configuration changed during capture",
            ));
        }
        Ok(())
    }
}

struct FrameTextures {
    input: Texture,
    legacy: Option<(Texture, Texture)>,
    format: DXGI_FORMAT,
}

impl FrameTextures {
    fn new(
        gpu: &Gpu,
        color: &ColorState,
        desc: &D3D11_TEXTURE2D_DESC,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let input = gpu.texture(desc.Width, desc.Height, desc.Format)?;
        let legacy = match color {
            ColorState::Advanced => None,
            ColorState::Legacy { .. } => Some((
                gpu.texture(desc.Width, desc.Height, DXGI_FORMAT_R32G32B32A32_FLOAT)?,
                gpu.texture(width, height, DXGI_FORMAT_R32G32B32A32_FLOAT)?,
            )),
        };
        Ok(Self {
            input,
            legacy,
            format: desc.Format,
        })
    }
}

struct AcquiredFrame {
    duplication: IDXGIOutputDuplication,
    texture: ID3D11Texture2D,
    released: bool,
}

impl AcquiredFrame {
    fn next(duplication: &IDXGIOutputDuplication) -> Result<Option<Self>> {
        unsafe {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;
            match duplication.AcquireNextFrame(0, &mut info, &mut resource) {
                Ok(()) => {}
                Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
                Err(error) => return Err(error),
            }
            let result = (|| {
                if info.ProtectedContentMaskedOut.as_bool() {
                    return Err(Error::new(
                        E_FAIL,
                        "Protected content is masked in this capture",
                    ));
                }
                resource
                    .ok_or_else(|| Error::new(E_FAIL, "Captured resource is required"))?
                    .cast::<ID3D11Texture2D>()
            })();
            match result {
                Ok(texture) => Ok(Some(Self {
                    duplication: duplication.clone(),
                    texture,
                    released: false,
                })),
                Err(error) => {
                    let _ = duplication.ReleaseFrame();
                    Err(error)
                }
            }
        }
    }

    fn release(mut self) -> Result<()> {
        self.released = true;
        unsafe { self.duplication.ReleaseFrame() }
    }

    #[cfg(test)]
    fn center_rgba8(&self, gpu: &Gpu) -> [u8; 4] {
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            self.texture.GetDesc(&mut desc);
            assert!(matches!(
                desc.Format,
                DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_R8G8B8A8_UNORM
            ));
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            gpu.device
                .CreateTexture2D(&desc, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            gpu.context.CopyResource(&staging, &self.texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            gpu.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let offset = (desc.Height / 2) as usize * mapped.RowPitch as usize
                + (desc.Width / 2) as usize * 4;
            let pixel = *(mapped.pData as *const u8).add(offset).cast::<[u8; 4]>();
            gpu.context.Unmap(&staging, 0);
            pixel
        }
    }
}

impl Drop for AcquiredFrame {
    fn drop(&mut self) {
        if !self.released {
            unsafe {
                let _ = self.duplication.ReleaseFrame();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FilterMode;

    #[test]
    #[ignore = "briefly shows a gray overlay on each legacy SDR display"]
    fn live_composition_is_visible_in_frozen_capture() {
        let class = WindowClass::new().unwrap();
        let factory = unsafe { CreateDXGIFactory1::<IDXGIFactory2>() }.unwrap();
        let specs = MonitorSpec::enumerate(&factory).unwrap();
        let mut verified = 0;
        for spec in specs {
            let color = ColorState::read(spec.adapter_id, &spec.desc.DeviceName).unwrap();
            if matches!(color, ColorState::Advanced) {
                continue;
            }
            let mut monitor = Monitor::new(
                &spec,
                color,
                &factory,
                &class,
                FilterMode::OklabGrayscale.fragment_shader(),
            )
            .unwrap();
            unsafe {
                let buffer = monitor.swap_chain.GetBuffer::<ID3D11Texture2D>(0).unwrap();
                let mut view = None;
                monitor
                    .gpu
                    .device
                    .CreateRenderTargetView(&buffer, None, Some(&mut view))
                    .unwrap();
                monitor
                    .gpu
                    .context
                    .ClearRenderTargetView(view.as_ref().unwrap(), &[0.18, 0.18, 0.18, 1.0]);
                monitor
                    .swap_chain
                    .Present(0, DXGI_PRESENT::default())
                    .ok()
                    .unwrap();
            }
            monitor.overlay.set_frozen(true).unwrap();
            monitor.overlay.show(true);
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut last = None;
            while Instant::now() < deadline {
                assert!(class.process_messages());
                if let Some(frame) = AcquiredFrame::next(&monitor.duplication).unwrap() {
                    last = Some(frame.center_rgba8(&monitor.gpu));
                    frame.release().unwrap();
                    if last.is_some_and(|pixel| pixel[..3] == [46, 46, 46]) {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            monitor.overlay.set_frozen(false).unwrap();
            monitor.overlay.show(false);
            assert!(
                last.is_some_and(|pixel| pixel[..3] == [46, 46, 46]),
                "{}: {last:?}",
                spec.name()
            );
            verified += 1;
        }
        assert!(verified > 0, "A legacy SDR display is required");
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop and capture permission"]
    fn live_desktop_capture_and_resource_restart() {
        let class = WindowClass::new().unwrap();
        for iteration in 0..2 {
            let factory = unsafe { CreateDXGIFactory1::<IDXGIFactory2>() }.unwrap();
            let specs = MonitorSpec::enumerate(&factory).unwrap();
            assert!(!specs.is_empty(), "An attached display is required");
            for spec in specs {
                let color = ColorState::read(spec.adapter_id, &spec.desc.DeviceName).unwrap();
                match &color {
                    ColorState::Advanced => eprintln!("{}: Advanced Color", spec.name()),
                    ColorState::Legacy { path, .. } => {
                        eprintln!("{}: ICC {}", spec.name(), path.display())
                    }
                }
                let mut monitor = Monitor::new(
                    &spec,
                    color,
                    &factory,
                    &class,
                    FilterMode::OklabGrayscale.fragment_shader(),
                )
                .unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                while !monitor.ready && Instant::now() < deadline {
                    assert!(class.process_messages());
                    monitor.capture().unwrap();
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(monitor.ready, "{} iteration {iteration}", spec.name());
                monitor.overlay.set_frozen(true).unwrap();
                monitor.overlay.set_frozen(false).unwrap();
            }
        }
    }
}
