# Screen Filter

A Windows desktop filter with Oklab and CIELAB grayscale modes.

## Grayscale definition

The Oklab mode converts the captured desktop into linear RGB with sRGB/Rec.709 primaries and a D65 white point. It computes Oklab, preserves `L`, and sets `a = b = 0`. The corresponding linear RGB neutral is `(L³, L³, L³)`.

Oklab estimates perceived lightness under its viewing assumptions. Display calibration, ambient light and visual adaptation affect physical perception.

The CIELAB mode preserves relative luminance `Y`, which preserves CIELAB `L*` when chroma is zeroed under the same reference white.

## Color management

Each display has its own capture device, duplication session, overlay and swap chain. Devices are created on the adapter that owns the display.

| Display mode | Processing |
| --- | --- |
| Legacy SDR | The active monitor's explicitly associated RGB ICC profile converts captured device RGB to linear scRGB. Grayscale is converted back through that profile before 8-bit presentation. |
| HDR or Advanced Color SDR | Capture must provide FP16 scRGB. The filter processes linear values directly and presents FP16 with the explicit scRGB color-space tag. Windows manages display conversion. |

Legacy ICC processing uses Direct2D color management, best quality, relative colorimetric intent and FP32 intermediate textures. Profile lookup follows the current user's association settings. Matrix/TRC and CLUT profiles use the Windows color engine.

Extended scRGB values, including negative components and values above one, remain available during grayscale calculation. HDR capture already contains the system's SDR-white scaling. That scale is preserved through the filter.

### Precision and support conditions

- The numerical reference is the composed desktop at capture time, interpreted through its active color configuration. Original source-image data belongs to the source application.
- The numerical contract applies to finite input colors and neutral results representable in the selected output format.
- Floating-point arithmetic, ICC tables and final display quantization determine numerical precision. Legacy SDR capture and presentation use 8 bits per channel. Advanced Color presentation uses FP16.
- Hardware calibration, Windows gamut mapping, HDR tone mapping and panel luminance limits determine the final physical output.
- Windows capture, profile lookup and presentation are separate API calls. Their synchronization defines an operating-system boundary. Color metadata is checked before processing and again before presentation.
- Legacy SDR requires an explicit, readable RGB display ICC association. Associate a suitable profile in Windows Color Management, including an sRGB profile for a characterized sRGB display.
- Advanced Color requires the executable's **Use legacy display ICC color management** compatibility option to be cleared. An exposed legacy standard profile while Advanced Color is active causes a pause for verification.
- Capture formats with an unverified encoding, missing profiles, conflicting display metadata, masked protected content and conversion errors pause the affected display. The overlay is removed and a notification and log entry describe the condition.
- Mirrored display paths require an independent-display configuration for this filter. Secure desktops and restricted capture sessions require capture access from Windows.

Display discovery and recovery run approximately once per second. Repeated identical errors share one notification until a valid frame is produced.

## Usage

Use Windows 10 version 2004 or later, a writable executable directory, and a Direct3D feature level 11.0 adapter. Advanced Color SDR depends on Windows 11's automatic color management support and display provisioning.

The first launch creates `config.toml` beside the executable:

```toml
toggle = "ctrl+alt+c"
freeze = "ctrl+alt+f"
mode = "OklabGrayscale"
launch_on_startup = true
refresh_rate = 0
```

- Toggle shows or hides the filter. Freeze retains the latest valid frame and includes the overlay in screen capture.
- `LabGrayscale` selects the CIELAB mode.
- `refresh_rate = 0` applies the capture polling cadence. A positive value sets an upper refresh-rate limit.
- Configuration edits and file replacements trigger reload. Invalid updates retain the active settings. Correcting an invalid initial configuration starts the filter.
- Configuration reloads and display-resource rebuilds acquire a new frame, including while freeze is selected.
- Screenshot interpretation follows the capturing application's pixel format and color metadata.
- `log.txt` and `panic.txt` are written beside the executable. Notification failures are logged.

## Structure

- `src/main.rs`: application ownership, hotkeys, worker completion and shutdown.
- `src/config.rs`: configuration validation and an owned file watcher.
- `src/render.rs`: per-display recovery, frame acquisition and presentation.
- `src/window.rs`: overlay windows, message processing and DPI scope.
- `src/gpu.rs`: D3D resources and shader compilation.
- `src/color.rs`: display color discovery and ICC conversions.
- `src/shaders/`: linear grayscale and display rotation.
- `src/gpu_tests.rs`: GPU readback against independent numerical references.

## Build and verification

```sh
cargo build --locked --release
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Run desktop and hardware tests explicitly, in sequence:

```sh
cargo test --locked -- --ignored --nocapture --test-threads=1
```

The desktop tests require an interactive Windows session and available hotkeys. The composition test briefly shows a gray overlay on each legacy SDR display.

See [verification details](docs/verification.md) for measured coverage, numerical thresholds and remaining hardware checks. ICC fixtures are committed test data. Their optional generator uses the Python standard library:

```sh
python tests/fixtures/generate_profiles.py
```

## References and credits

- [Oklab definition and reference matrices](https://bottosson.github.io/posts/oklab/)
- [Windows Advanced Color and scRGB](https://learn.microsoft.com/en-us/windows/win32/direct3darticles/high-dynamic-range)
- [ICC behavior with Advanced Color](https://learn.microsoft.com/en-us/windows/win32/wcs/advanced-color-icc-profiles)
- [Direct2D color management](https://learn.microsoft.com/en-us/windows/win32/direct2d/color-management)
- [Desktop duplication formats](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_5/nf-dxgi1_5-idxgioutput5-duplicateoutput1)
- Color conversion code derived from [Bevy](https://bevy.org/), under the MIT license.
