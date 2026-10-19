# Verification

## Environment

- Windows 11 Pro, build 22631, x64.
- Rust 1.96.0, MSVC target.
- NVIDIA GeForce RTX 4060 Laptop GPU.
- AMD Radeon 780M Graphics.
- Two attached legacy SDR displays with distinct associated ICC profiles.

## Commands and results

| Command | Result |
| --- | --- |
| `cargo test --locked -- --nocapture` | 20 passed. Four hardware/desktop tests require explicit invocation. |
| `cargo test --locked -- --ignored --nocapture --test-threads=1` | Four passed. |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed. |
| `cargo build --locked --release` | Passed. |
| `cargo fmt -- --check` | Passed. |

The standard suite uses WARP to run the compiled HLSL with GPU readback. The explicit hardware suite runs the Oklab vectors on both adapters. The desktop suite captures both attached displays twice, verifies ICC conversion and presentation, and recreates capture resources. A composition test shows a known gray overlay and verifies its pixels through frozen desktop capture. The hotkey test starts and joins three worker groups with the same hotkeys.

Configuration tests cover invalid hotkeys, duplicate bindings, unknown fields, invalid initial settings, file replacement, recovery and watcher shutdown.

## Numerical checks

The Oklab test contains a `17 × 17 × 17` SDR RGB grid and eight additional vectors for signed values, Display P3 colors represented in linear sRGB, HDR values and near-black values. The independent CPU reference uses `f64` and signed `cbrt`.

The error measure is:

```text
abs(gpu_gray - reference_gray) / max(1, abs(reference_gray))
```

The assertion threshold is `2e-5`. Measured maxima:

| Adapter | Maximum normalized error |
| --- | --- |
| WARP | 5.203e-7 |
| NVIDIA RTX 4060 Laptop | 3.541e-7 |
| AMD Radeon 780M | 3.402e-7 |

Further assertions cover identical linear gray channels, opaque output, neutral preservation, all four rotations, CIELAB relative luminance, and FP16 extended-range input and output. FP16 tests include its quantization tolerance.

ICC tests use generated, committed fixtures:

- sRGB matrix with piecewise parametric transfer curves.
- Display P3 matrix with sRGB transfer curves. Negative and greater-than-one linear sRGB values are checked before grayscale.
- Distinct channel gamma curves, including device encoding and final 8-bit quantization.
- A LUT16/CLUT profile with forward and reverse transforms.

ICC component assertions use an absolute `2e-4` tolerance for those fixture vectors. The measured maximum sRGB linear decoding error is `3.061e-5`. Integer output assertions include half a code value plus the ICC conversion tolerance. These thresholds describe the exercised vectors and profiles. Profile characterization, interpolation and complex profile tables introduce their own error.

Metadata tests cover HDR/WCG flag interpretation, conflicting flags, explicit legacy profile requirements, the Advanced Color legacy-profile guard, and capture-format rejection.

## Remaining hardware acceptance

The following scenarios require dedicated environments:

- Actual HDR desktop capture, HDR brightness changes and HDR-to-SDR transitions.
- Advanced Color SDR with automatic color management enabled, including the legacy ICC compatibility guard on a real Advanced Color display.
- Mixed HDR/SDR displays across adapters.
- Windows 10 runtime behavior.
- Physical display rotation, mixed DPI, display hot-plug, sleep/resume, secure-desktop transitions and driver removal.

The implementation handles these through color-state validation and per-display resource recreation. Their real-device behavior remains a hardware acceptance item. The synthetic FP16 checks establish shader and storage behavior separately from HDR desktop capture.

Physical colorimeter measurements belong to display calibration validation. Windows tone mapping, output gamut limits and panel behavior remain part of the physical display chain.
