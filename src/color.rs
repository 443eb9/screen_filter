use std::{ffi::OsString, mem::size_of, os::windows::ffi::OsStringExt, path::PathBuf, slice};

use windows::{
    Win32::{
        Devices::Display::*,
        Foundation::{E_FAIL, ERROR_INSUFFICIENT_BUFFER, LUID},
        Graphics::{
            Direct2D::{Common::*, *},
            Direct3D11::ID3D11Texture2D,
            Dxgi::{Common::*, *},
            Gdi::*,
        },
        UI::{ColorSystem::*, WindowsAndMessaging::EDD_GET_DEVICE_INTERFACE_NAME},
    },
    core::{BOOL, Error, Interface, PCWSTR, PWSTR, Result},
};

#[derive(Clone, Debug, PartialEq)]
pub enum ColorState {
    Advanced,
    Legacy { path: PathBuf, profile: Vec<u8> },
}

impl ColorState {
    pub fn read(adapter_id: LUID, name: &[u16]) -> Result<Self> {
        if Self::advanced_color_enabled(adapter_id, name)? {
            return Ok(Self::Advanced);
        }

        let path = Self::profile_path(name)?;
        let profile = std::fs::read(&path)
            .map_err(|error| Error::new(E_FAIL, format!("ICC {}: {error}", path.display())))?;
        if profile.len() < 128
            || &profile[36..40] != b"acsp"
            || &profile[16..20] != b"RGB "
            || &profile[12..16] != b"mntr"
        {
            return Err(Error::new(E_FAIL, "An RGB ICC display profile is required"));
        }
        Ok(Self::Legacy { path, profile })
    }

    fn profile_path(name: &[u16]) -> Result<PathBuf> {
        unsafe {
            let mut monitor = DISPLAY_DEVICEW {
                cb: size_of::<DISPLAY_DEVICEW>() as u32,
                ..Default::default()
            };
            EnumDisplayDevicesW(
                PCWSTR(name.as_ptr()),
                0,
                &mut monitor,
                EDD_GET_DEVICE_INTERFACE_NAME,
            )
            .ok()?;
            let key = PCWSTR(monitor.DeviceKey.as_ptr());
            // CLASS_MONITOR from icm.h, omitted from the Windows metadata bindings.
            const CLASS_MONITOR: u32 = u32::from_be_bytes(*b"mntr");
            let mut per_user = BOOL::default();
            WcsGetUsePerUserProfiles(key, CLASS_MONITOR, &mut per_user).ok()?;
            let scope = if per_user.as_bool() {
                WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER
            } else {
                WCS_PROFILE_MANAGEMENT_SCOPE_SYSTEM_WIDE
            };
            let mut bytes = 0;
            WcsGetDefaultColorProfileSize(
                scope,
                key,
                CPT_ICC,
                CPST_STANDARD_DISPLAY_COLOR_MODE,
                0,
                &mut bytes,
            )
            .ok()?;
            if bytes == 0 || bytes % 2 != 0 {
                return Err(Error::new(
                    E_FAIL,
                    "A display ICC profile association is required",
                ));
            }
            let mut filename = vec![0; bytes as usize / 2];
            WcsGetDefaultColorProfile(
                scope,
                key,
                CPT_ICC,
                CPST_STANDARD_DISPLAY_COLOR_MODE,
                0,
                bytes,
                PWSTR(filename.as_mut_ptr()),
            )
            .ok()?;
            let end = filename
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(filename.len());
            let path = PathBuf::from(OsString::from_wide(&filename[..end]));
            if path.is_absolute() {
                return Ok(path);
            }

            let mut bytes = 0;
            let _ = GetColorDirectoryW(None, None, &mut bytes);
            if bytes == 0 {
                return Err(Error::from_win32());
            }
            let mut directory = vec![0; bytes as usize / 2];
            GetColorDirectoryW(None, Some(PWSTR(directory.as_mut_ptr())), &mut bytes).ok()?;
            let end = directory
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(directory.len());
            Ok(PathBuf::from(OsString::from_wide(&directory[..end])).join(path))
        }
    }

    fn advanced_color_enabled(adapter_id: LUID, name: &[u16]) -> Result<bool> {
        unsafe {
            // The desktop may change between sizing and querying. Retry only that documented race.
            let mut attempts = 3;
            let paths = loop {
                let mut path_count = 0;
                let mut mode_count = 0;
                GetDisplayConfigBufferSizes(
                    QDC_ONLY_ACTIVE_PATHS,
                    &mut path_count,
                    &mut mode_count,
                )
                .ok()?;
                let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
                let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
                let status = QueryDisplayConfig(
                    QDC_ONLY_ACTIVE_PATHS,
                    &mut path_count,
                    paths.as_mut_ptr(),
                    &mut mode_count,
                    modes.as_mut_ptr(),
                    None,
                );
                if status == ERROR_INSUFFICIENT_BUFFER {
                    attempts -= 1;
                    if attempts == 0 {
                        return Err(Error::new(
                            E_FAIL,
                            "Display configuration kept changing during query",
                        ));
                    }
                    continue;
                }
                status.ok()?;
                paths.truncate(path_count as usize);
                break paths;
            };
            let mut active = None;
            for path in paths {
                if path.sourceInfo.adapterId != adapter_id {
                    continue;
                }
                let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                    header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                        size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                        adapterId: adapter_id,
                        id: path.sourceInfo.id,
                    },
                    ..Default::default()
                };
                let status = DisplayConfigGetDeviceInfo(&mut source.header);
                if status != 0 {
                    return Err(Error::from_hresult(windows::core::HRESULT::from_win32(
                        status as u32,
                    )));
                }
                if source.viewGdiDeviceName != name {
                    continue;
                }
                if active.is_some() {
                    return Err(Error::new(
                        E_FAIL,
                        "Cloned display paths require independent display configuration",
                    ));
                }
                let mut info = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
                    header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                        size: size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32,
                        adapterId: path.targetInfo.adapterId,
                        id: path.targetInfo.id,
                    },
                    ..Default::default()
                };
                let status = DisplayConfigGetDeviceInfo(&mut info.header);
                if status != 0 {
                    return Err(Error::from_hresult(windows::core::HRESULT::from_win32(
                        status as u32,
                    )));
                }
                const ADVANCED_COLOR_ENABLED: u32 = 1 << 1;
                active = Some(info.Anonymous.value & ADVANCED_COLOR_ENABLED != 0);
            }
            active
                .ok_or_else(|| Error::new(E_FAIL, "Active display color configuration is required"))
        }
    }

    pub fn validate_format(&self, format: DXGI_FORMAT) -> Result<()> {
        let valid = match self {
            Self::Advanced => format == DXGI_FORMAT_R16G16B16A16_FLOAT,
            Self::Legacy { .. } => matches!(
                format,
                DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_R8G8B8A8_UNORM
            ),
        };
        if valid {
            Ok(())
        } else {
            Err(Error::new(
                E_FAIL,
                format!("Capture format {format:?} has an unverified color encoding"),
            ))
        }
    }

    pub fn output_format(&self) -> DXGI_FORMAT {
        match self {
            Self::Advanced => DXGI_FORMAT_R16G16B16A16_FLOAT,
            Self::Legacy { .. } => DXGI_FORMAT_B8G8R8A8_UNORM,
        }
    }
}

pub struct ColorTransform {
    context: ID2D1DeviceContext,
    to_linear: ID2D1Effect,
    to_display: ID2D1Effect,
}

impl ColorTransform {
    pub fn new(device: &IDXGIDevice, profile: &[u8]) -> Result<Self> {
        unsafe {
            let context = D2D1CreateDevice(device, None)?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            if !context
                .IsBufferPrecisionSupported(D2D1_BUFFER_PRECISION_32BPC_FLOAT)
                .as_bool()
            {
                return Err(Error::new(
                    E_FAIL,
                    "ICC processing requires 32-bit floating-point precision",
                ));
            }
            context.SetRenderingControls(&D2D1_RENDERING_CONTROLS {
                bufferPrecision: D2D1_BUFFER_PRECISION_32BPC_FLOAT,
                ..Default::default()
            });
            let display = context
                .CreateColorContext(D2D1_COLOR_SPACE_CUSTOM, Some(profile))
                .map_err(|error| {
                    Error::new(error.code(), format!("ICC profile context: {error}"))
                })?;
            let linear = context.CreateColorContext(D2D1_COLOR_SPACE_SCRGB, None)?;
            let to_linear = Self::effect(&context, &display, &linear).map_err(|error| {
                Error::new(error.code(), format!("ICC decoding effect: {error}"))
            })?;
            let to_display = Self::effect(&context, &linear, &display).map_err(|error| {
                Error::new(error.code(), format!("ICC encoding effect: {error}"))
            })?;
            Ok(Self {
                context,
                to_linear,
                to_display,
            })
        }
    }

    fn effect(
        context: &ID2D1DeviceContext,
        source: &ID2D1ColorContext,
        destination: &ID2D1ColorContext,
    ) -> Result<ID2D1Effect> {
        unsafe {
            let effect = context.CreateEffect(&CLSID_D2D1ColorManagement)?;
            for (property, color_context) in [
                (D2D1_COLORMANAGEMENT_PROP_SOURCE_COLOR_CONTEXT, source),
                (
                    D2D1_COLORMANAGEMENT_PROP_DESTINATION_COLOR_CONTEXT,
                    destination,
                ),
            ] {
                let raw = color_context.as_raw();
                effect
                    .SetValue(
                        property.0 as u32,
                        D2D1_PROPERTY_TYPE_COLOR_CONTEXT,
                        slice::from_raw_parts(
                            (&raw as *const *mut std::ffi::c_void).cast(),
                            size_of::<*mut std::ffi::c_void>(),
                        ),
                    )
                    .map_err(|error| {
                        Error::new(
                            error.code(),
                            format!("Color context property {property:?}: {error}"),
                        )
                    })?;
            }
            for (property, value) in [
                (
                    D2D1_COLORMANAGEMENT_PROP_SOURCE_RENDERING_INTENT,
                    D2D1_COLORMANAGEMENT_RENDERING_INTENT_RELATIVE_COLORIMETRIC.0,
                ),
                (
                    D2D1_COLORMANAGEMENT_PROP_DESTINATION_RENDERING_INTENT,
                    D2D1_COLORMANAGEMENT_RENDERING_INTENT_RELATIVE_COLORIMETRIC.0,
                ),
                (
                    D2D1_COLORMANAGEMENT_PROP_QUALITY,
                    D2D1_COLORMANAGEMENT_QUALITY_BEST.0,
                ),
                (
                    D2D1_COLORMANAGEMENT_PROP_ALPHA_MODE,
                    D2D1_COLORMANAGEMENT_ALPHA_MODE_STRAIGHT.0,
                ),
            ] {
                effect
                    .SetValue(
                        property.0 as u32,
                        D2D1_PROPERTY_TYPE_ENUM,
                        &value.to_ne_bytes(),
                    )
                    .map_err(|error| {
                        Error::new(
                            error.code(),
                            format!("Color conversion property {property:?}: {error}"),
                        )
                    })?;
            }
            effect
                .SetValue(
                    D2D1_PROPERTY_PRECISION.0 as u32,
                    D2D1_PROPERTY_TYPE_ENUM,
                    &D2D1_BUFFER_PRECISION_32BPC_FLOAT.0.to_ne_bytes(),
                )
                .map_err(|error| {
                    Error::new(error.code(), format!("Color conversion precision: {error}"))
                })?;
            Ok(effect)
        }
    }

    fn bitmap(&self, texture: &ID3D11Texture2D, target: bool) -> Result<ID2D1Bitmap1> {
        unsafe {
            let surface = texture.cast::<IDXGISurface>()?;
            self.context.CreateBitmapFromDxgiSurface(
                &surface,
                Some(&D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_UNKNOWN,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: if target {
                        D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW
                    } else {
                        D2D1_BITMAP_OPTIONS_NONE
                    },
                    ..Default::default()
                }),
            )
        }
    }

    fn convert(
        &self,
        effect: &ID2D1Effect,
        input: &ID3D11Texture2D,
        output: &ID3D11Texture2D,
    ) -> Result<()> {
        unsafe {
            let source = self
                .bitmap(input, false)
                .map_err(|error| Error::new(error.code(), format!("ICC input bitmap: {error}")))?;
            let target = self
                .bitmap(output, true)
                .map_err(|error| Error::new(error.code(), format!("ICC target bitmap: {error}")))?;
            effect.SetInput(0, &source, true);
            let image = effect.GetOutput()?;
            self.context.SetTarget(&target);
            self.context.BeginDraw();
            self.context.DrawImage(
                &image,
                None,
                None,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                D2D1_COMPOSITE_MODE_SOURCE_COPY,
            );
            let result = self.context.EndDraw(None, None);
            self.context.SetTarget(None);
            effect.SetInput(0, None, true);
            result.map_err(|error| {
                Error::new(error.code(), format!("ICC conversion drawing: {error}"))
            })
        }
    }

    pub fn decode(&self, input: &ID3D11Texture2D, output: &ID3D11Texture2D) -> Result<()> {
        self.convert(&self.to_linear, input, output)
            .map_err(|error| Error::new(error.code(), format!("ICC decoding: {error}")))
    }

    pub fn encode(&self, input: &ID3D11Texture2D, output: &ID3D11Texture2D) -> Result<()> {
        self.convert(&self.to_display, input, output)
            .map_err(|error| Error::new(error.code(), format!("ICC encoding: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advanced_color_requires_float_capture() {
        assert!(
            ColorState::Advanced
                .validate_format(DXGI_FORMAT_R16G16B16A16_FLOAT)
                .is_ok()
        );
        for format in [
            DXGI_FORMAT_B8G8R8A8_UNORM,
            DXGI_FORMAT_R10G10B10A2_UNORM,
            DXGI_FORMAT_UNKNOWN,
        ] {
            assert!(ColorState::Advanced.validate_format(format).is_err());
        }
    }

    #[test]
    fn legacy_capture_requires_verified_integer_encoding() {
        let color = ColorState::Legacy {
            path: PathBuf::new(),
            profile: Vec::new(),
        };
        assert!(color.validate_format(DXGI_FORMAT_B8G8R8A8_UNORM).is_ok());
        assert!(color.validate_format(DXGI_FORMAT_R8G8B8A8_UNORM).is_ok());
        assert!(
            color
                .validate_format(DXGI_FORMAT_R16G16B16A16_FLOAT)
                .is_err()
        );
    }
}
