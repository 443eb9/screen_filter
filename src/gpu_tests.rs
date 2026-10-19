use std::{array, slice};

use windows::{
    Win32::Graphics::{
        Direct3D::D3D_DRIVER_TYPE_WARP,
        Direct3D11::*,
        Dxgi::{Common::*, IDXGIDevice},
    },
    core::Interface,
};

use crate::{
    color::ColorTransform,
    config::FilterMode,
    gpu::{Gpu, Texture},
};

struct TestGpu {
    gpu: Gpu,
}

impl TestGpu {
    fn new(mode: FilterMode) -> Self {
        Self {
            gpu: Gpu::new(None, D3D_DRIVER_TYPE_WARP, mode.fragment_shader()).unwrap(),
        }
    }

    fn upload(&self, width: u32, height: u32, pixels: &[[f32; 4]]) -> Texture {
        assert_eq!(pixels.len(), (width * height) as usize);
        let texture = self
            .gpu
            .texture(width, height, DXGI_FORMAT_R32G32B32A32_FLOAT)
            .unwrap();
        unsafe {
            self.gpu.context.UpdateSubresource(
                &texture.raw,
                0,
                None,
                pixels.as_ptr().cast(),
                width * 16,
                0,
            );
        }
        texture
    }

    fn bytes(&self, texture: &ID3D11Texture2D, pixel_size: usize) -> Vec<u8> {
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            self.gpu
                .device
                .CreateTexture2D(&desc, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            self.gpu.context.CopyResource(&staging, texture);
            let row_bytes = desc.Width as usize * pixel_size;
            let mut bytes = vec![0; row_bytes * desc.Height as usize];
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.gpu
                .context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            for y in 0..desc.Height as usize {
                let row = slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    row_bytes,
                );
                bytes[y * row_bytes..(y + 1) * row_bytes].copy_from_slice(row);
            }
            self.gpu.context.Unmap(&staging, 0);
            bytes
        }
    }

    fn rgba32(&self, texture: &ID3D11Texture2D) -> Vec<[f32; 4]> {
        self.bytes(texture, 16)
            .chunks_exact(16)
            .map(|pixel| {
                array::from_fn(|channel| {
                    f32::from_ne_bytes(pixel[channel * 4..channel * 4 + 4].try_into().unwrap())
                })
            })
            .collect()
    }

    fn draw(
        &self,
        pixels: &[[f32; 4]],
        width: u32,
        height: u32,
        rotation: DXGI_MODE_ROTATION,
    ) -> Vec<[f32; 4]> {
        let input = self.upload(width, height, pixels);
        let (width, height) = if matches!(
            rotation,
            DXGI_MODE_ROTATION_ROTATE90 | DXGI_MODE_ROTATION_ROTATE270
        ) {
            (height, width)
        } else {
            (width, height)
        };
        let output = self
            .gpu
            .texture(width, height, DXGI_FORMAT_R32G32B32A32_FLOAT)
            .unwrap();
        self.gpu.draw(&input, &output.raw, rotation).unwrap();
        self.rgba32(&output.raw)
    }

    fn transform(&self, profile: &[u8]) -> ColorTransform {
        ColorTransform::new(&self.gpu.device.cast::<IDXGIDevice>().unwrap(), profile).unwrap()
    }
}

fn oklab(rgb: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = rgb;
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

#[test]
fn oklab_reference_matches_published_primaries() {
    for (rgb, expected) in [
        ([1.0, 0.0, 0.0], [0.6279553606, 0.2248630611, 0.1258462985]),
        ([0.0, 1.0, 0.0], [0.8664396115, -0.2338875742, 0.1794984799]),
        (
            [0.0, 0.0, 1.0],
            [0.4520137184, -0.0324569842, -0.3115281477],
        ),
    ] {
        for (actual, expected) in oklab(rgb).into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-9);
        }
    }
}

#[test]
fn oklab_shader_matches_f64_reference_for_sdr_hdr_and_signed_values() {
    TestGpu::new(FilterMode::OklabGrayscale).verify_oklab();
}

impl TestGpu {
    fn verify_oklab(&self) {
        let mut inputs = Vec::new();
        for r in 0..17 {
            for g in 0..17 {
                for b in 0..17 {
                    inputs.push([r as f32 / 16.0, g as f32 / 16.0, b as f32 / 16.0, 1.0]);
                }
            }
        }
        inputs.extend([
            [-0.25, -0.25, -0.25, 1.0],
            [-0.25, 0.1, 0.2, 1.0],
            [1.22494, -0.042057, -0.019638, 1.0],
            [-0.22494, 1.042057, -0.078636, 1.0],
            [12.5, 2.0, 0.1, 1.0],
            [125.0, 125.0, 125.0, 1.0],
            [1e-12, 1e-12, 1e-12, 1.0],
            [0.0031308, 0.0031308, 0.0031308, 1.0],
        ]);
        let result = self.draw(&inputs, inputs.len() as u32, 1, DXGI_MODE_ROTATION_IDENTITY);
        let mut max_error = 0.0f64;
        for (input, output) in inputs.iter().zip(result) {
            let expected = oklab([input[0] as f64, input[1] as f64, input[2] as f64])[0].powi(3);
            let error = (output[0] as f64 - expected).abs() / expected.abs().max(1.0);
            max_error = max_error.max(error);
            assert!(output[0].is_finite(), "{input:?}: {output:?}");
            assert_eq!(output[0], output[1]);
            assert_eq!(output[1], output[2]);
            assert_eq!(output[3], 1.0);
            assert!(
                error < 2e-5,
                "{input:?}: {output:?}, expected {expected}, error {error}"
            );
        }
        eprintln!("Oklab shader maximum normalized error: {max_error:.9e}");
    }
}

#[test]
#[ignore = "requires hardware Direct3D adapters"]
fn hardware_adapters_match_oklab_reference() {
    use windows::Win32::Graphics::{
        Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
        Dxgi::{CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, IDXGIAdapter, IDXGIFactory2},
    };

    let factory = unsafe { CreateDXGIFactory1::<IDXGIFactory2>() }.unwrap();
    let mut index = 0;
    let mut verified = 0;
    loop {
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => panic!("Adapter enumeration: {error}"),
        };
        let desc = unsafe { adapter.GetDesc1() }.unwrap();
        if desc.Flags & windows::Win32::Graphics::Dxgi::DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0 {
            let name = String::from_utf16_lossy(&desc.Description);
            eprintln!("Adapter: {}", name.trim_end_matches('\0'));
            let adapter = adapter.cast::<IDXGIAdapter>().unwrap();
            let test = TestGpu {
                gpu: Gpu::new(
                    Some(&adapter),
                    D3D_DRIVER_TYPE_UNKNOWN,
                    FilterMode::OklabGrayscale.fragment_shader(),
                )
                .unwrap(),
            };
            test.verify_oklab();
            verified += 1;
        }
        index += 1;
    }
    assert!(verified > 0, "A hardware adapter is required");
}

#[test]
fn grayscale_preserves_neutrals_and_rotates_without_interpolation() {
    let test = TestGpu::new(FilterMode::OklabGrayscale);
    let pixels = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6].map(|gray| [gray, gray, gray, 1.0]);
    for (rotation, order) in [
        (DXGI_MODE_ROTATION_IDENTITY, [0, 1, 2, 3, 4, 5]),
        (DXGI_MODE_ROTATION_ROTATE90, [3, 0, 4, 1, 5, 2]),
        (DXGI_MODE_ROTATION_ROTATE180, [5, 4, 3, 2, 1, 0]),
        (DXGI_MODE_ROTATION_ROTATE270, [2, 5, 1, 4, 0, 3]),
    ] {
        let result = test.draw(&pixels, 3, 2, rotation);
        for (pixel, index) in result.iter().zip(order) {
            assert!(
                (pixel[0] - pixels[index][0]).abs() < 2e-5,
                "{rotation:?}: {result:?}"
            );
        }
    }
}

#[test]
fn lab_shader_preserves_relative_luminance() {
    let test = TestGpu::new(FilterMode::LabGrayscale);
    let pixels = [
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
        [0.18, 0.18, 0.18, 1.0],
    ];
    let result = test.draw(&pixels, 4, 1, DXGI_MODE_ROTATION_IDENTITY);
    for (input, output) in pixels.iter().zip(result) {
        let expected =
            input[0] as f64 * 0.2126729 + input[1] as f64 * 0.7151522 + input[2] as f64 * 0.0721750;
        assert!((output[0] as f64 - expected).abs() < 1e-6);
    }
}

#[test]
fn fp16_scrgb_capture_and_output_preserve_extended_range() {
    let test = TestGpu::new(FilterMode::OklabGrayscale);
    let pixels = [
        [0x3c00u16, 0, 0, 0x3c00],
        [0xb400, 0x3c00, 0x3800, 0x3c00],
        [0x4a40, 0x4a40, 0x4a40, 0x3c00],
        [0x57d0, 0x57d0, 0x57d0, 0x3c00],
        [0x1400, 0x1400, 0x1400, 0x3c00],
    ];
    let decode = |bits: u16| {
        let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
        let exponent = ((bits >> 10) & 31) as i32;
        let fraction = (bits & 1023) as f64;
        if exponent == 0 {
            sign * fraction * 2.0f64.powi(-24)
        } else {
            sign * (1.0 + fraction / 1024.0) * 2.0f64.powi(exponent - 15)
        }
    };
    let input = test
        .gpu
        .texture(pixels.len() as u32, 1, DXGI_FORMAT_R16G16B16A16_FLOAT)
        .unwrap();
    let output = test
        .gpu
        .texture(pixels.len() as u32, 1, DXGI_FORMAT_R16G16B16A16_FLOAT)
        .unwrap();
    unsafe {
        test.gpu.context.UpdateSubresource(
            &input.raw,
            0,
            None,
            pixels.as_ptr().cast(),
            pixels.len() as u32 * 8,
            0,
        );
    }
    test.gpu
        .draw(&input, &output.raw, DXGI_MODE_ROTATION_IDENTITY)
        .unwrap();
    let bytes = test.bytes(&output.raw, 8);
    for (input, output) in pixels.iter().zip(bytes.chunks_exact(8)) {
        let expected = oklab([decode(input[0]), decode(input[1]), decode(input[2])])[0].powi(3);
        let gray = decode(u16::from_ne_bytes(output[0..2].try_into().unwrap()));
        assert!(
            (gray - expected).abs() < expected.abs().max(1e-4) * 0.001,
            "{gray} vs {expected}"
        );
        assert_eq!(&output[0..2], &output[2..4]);
        assert_eq!(&output[2..4], &output[4..6]);
    }
}

#[test]
fn icc_srgb_decoding_matches_piecewise_transfer() {
    let test = TestGpu::new(FilterMode::OklabGrayscale);
    let transform = test.transform(include_bytes!("../tests/fixtures/srgb.icc"));
    let pixels = (0..=256)
        .map(|i| {
            let x = i as f32 / 256.0;
            [x, 1.0 - x, x, 1.0]
        })
        .collect::<Vec<_>>();
    let input = test.upload(pixels.len() as u32, 1, &pixels);
    let linear = test
        .gpu
        .texture(pixels.len() as u32, 1, DXGI_FORMAT_R32G32B32A32_FLOAT)
        .unwrap();
    transform.decode(&input.raw, &linear.raw).unwrap();
    let mut max_error = 0.0f64;
    for (input, output) in pixels.iter().zip(test.rgba32(&linear.raw)) {
        for channel in 0..3 {
            let x = input[channel] as f64;
            let expected = if x <= 0.04045 {
                x / 12.92
            } else {
                ((x + 0.055) / 1.055).powf(2.4)
            };
            let error = (output[channel] as f64 - expected).abs();
            max_error = max_error.max(error);
            assert!(error < 2e-4, "{input:?}: {output:?}, expected {expected}");
        }
    }
    eprintln!("ICC sRGB maximum absolute linear error: {max_error:.9e}");
}

#[test]
fn icc_display_p3_preserves_wide_gamut_before_grayscale() {
    let test = TestGpu::new(FilterMode::OklabGrayscale);
    let transform = test.transform(include_bytes!("../tests/fixtures/display_p3.icc"));
    let input = test.upload(
        3,
        1,
        &[
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        ],
    );
    let linear = test
        .gpu
        .texture(3, 1, DXGI_FORMAT_R32G32B32A32_FLOAT)
        .unwrap();
    let gray = test
        .gpu
        .texture(3, 1, DXGI_FORMAT_R32G32B32A32_FLOAT)
        .unwrap();
    transform.decode(&input.raw, &linear.raw).unwrap();
    let expected = [
        [1.22494018, -0.04205695, -0.01963755],
        [-0.22494018, 1.04205695, -0.07863605],
        [0.0, 0.0, 1.09827360],
    ];
    for (actual, expected) in test.rgba32(&linear.raw).iter().zip(expected) {
        for channel in 0..3 {
            assert!(
                (actual[channel] as f64 - expected[channel]).abs() < 2e-4,
                "{actual:?} vs {expected:?}"
            );
        }
    }
    test.gpu
        .draw(&linear, &gray.raw, DXGI_MODE_ROTATION_IDENTITY)
        .unwrap();
    for (actual, rgb) in test.rgba32(&gray.raw).iter().zip(expected) {
        let expected = oklab(rgb)[0].powi(3);
        assert!((actual[0] as f64 - expected).abs() < 2e-4);
    }
}

#[test]
fn icc_output_applies_channel_calibration_and_integer_quantization() {
    let test = TestGpu::new(FilterMode::OklabGrayscale);
    let transform = test.transform(include_bytes!("../tests/fixtures/channel_gamma.icc"));
    let linear = test.upload(1, 1, &[[0.18, 0.18, 0.18, 1.0]]);
    let encoded = test
        .gpu
        .texture(1, 1, DXGI_FORMAT_R32G32B32A32_FLOAT)
        .unwrap();
    let integer = test.gpu.texture(1, 1, DXGI_FORMAT_B8G8R8A8_UNORM).unwrap();
    let decoded = test
        .gpu
        .texture(1, 1, DXGI_FORMAT_R32G32B32A32_FLOAT)
        .unwrap();
    transform.encode(&linear.raw, &encoded.raw).unwrap();
    transform.encode(&linear.raw, &integer.raw).unwrap();
    let actual = test.rgba32(&encoded.raw)[0];
    let bytes = test.bytes(&integer.raw, 4);
    for (channel, gamma) in [1.8, 2.2, 2.6].into_iter().enumerate() {
        let expected = 0.18f64.powf(1.0 / gamma);
        assert!(
            (actual[channel] as f64 - expected).abs() < 2e-4,
            "{actual:?} vs {expected}"
        );
        assert!((bytes[2 - channel] as f64 / 255.0 - expected).abs() <= 0.5 / 255.0 + 2e-4);
    }
    transform.decode(&encoded.raw, &decoded.raw).unwrap();
    let roundtrip = test.rgba32(&decoded.raw)[0];
    for value in &roundtrip[..3] {
        assert!((*value - 0.18).abs() < 2e-4, "{roundtrip:?}");
    }
}

#[test]
fn icc_clut_profile_converts_in_both_directions() {
    let test = TestGpu::new(FilterMode::OklabGrayscale);
    let transform = test.transform(include_bytes!("../tests/fixtures/lut.icc"));
    let input = test.upload(1, 1, &[[0.75, 0.6, 0.5, 1.0]]);
    let linear = test
        .gpu
        .texture(1, 1, DXGI_FORMAT_R32G32B32A32_FLOAT)
        .unwrap();
    let encoded = test
        .gpu
        .texture(1, 1, DXGI_FORMAT_R32G32B32A32_FLOAT)
        .unwrap();
    transform.decode(&input.raw, &linear.raw).unwrap();
    let actual = test.rgba32(&linear.raw)[0];
    for (value, input) in actual[..3].iter().zip([0.75f64, 0.6, 0.5]) {
        let expected = input.powf(2.2);
        assert!(
            (*value as f64 - expected).abs() < 2e-4,
            "{actual:?}, expected {expected}"
        );
    }
    transform.encode(&linear.raw, &encoded.raw).unwrap();
    let actual = test.rgba32(&encoded.raw)[0];
    for (value, expected) in actual[..3].iter().zip([0.75f32, 0.6, 0.5]) {
        assert!((*value - expected).abs() < 2e-4, "{actual:?}");
    }
}
