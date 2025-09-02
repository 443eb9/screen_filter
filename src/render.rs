#![allow(non_snake_case)]

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::Fxc::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

pub static ENABLED: AtomicBool = AtomicBool::new(false);

#[repr(C)]
struct SimpleVertex {
    x: f32,
    y: f32,
    u: f32,
    v: f32,
}

struct OutputDup {
    dup: IDXGIOutputDuplication,
    desktop_rect: RECT,
    width: u32,
    height: u32,
    dest_tex: ID3D11Texture2D,
    dest_srv: ID3D11ShaderResourceView,
}

struct Globals {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain,
    rtv: ID3D11RenderTargetView,
    vs: ID3D11VertexShader,
    input_layout: ID3D11InputLayout,
    vb: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    outputs: Vec<OutputDup>,
}

struct FragmentShader {
    ps: ID3D11PixelShader,
}

unsafe extern "system" fn WndProc(hWnd: HWND, msg: u32, wParam: WPARAM, lParam: LPARAM) -> LRESULT {
    match msg {
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcA(hWnd, msg, wParam, lParam) },
    }
}

const VS_SRC: &str = r#"
void main(in float2 pos : POSITION, in float2 tex : TEXCOORD,
          out float4 outPos : SV_Position, out float2 outTex : TEXCOORD)
{
    outPos = float4(pos, 0.0f, 1.0f);
    outTex = tex;
}
"#;

pub fn render_loop(fragment: &'static str) -> windows::core::Result<()> {
    unsafe {
        let hinstance = GetModuleHandleA(None)?;
        let class_name = s!("DX11ScreenFilter");

        let wc = WNDCLASSEXA {
            cbSize: size_of::<WNDCLASSEXA>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(WndProc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassExA(&wc);

        let virt_left = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let virt_top = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let virt_w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let virt_h = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        let hWnd = CreateWindowExA(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name,
            s!("Screen Filter"),
            WS_POPUP,
            virt_left,
            virt_top,
            virt_w,
            virt_h,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;

        let _ = SetWindowDisplayAffinity(hWnd, WDA_EXCLUDEFROMCAPTURE);
        SetLayeredWindowAttributes(hWnd, COLORREF(0), 255, LWA_ALPHA)?;

        let mut g = init_d3d11(hWnd)?;
        init_duplications(&mut g)?;
        let frag = compile_shader(&g, fragment)?;

        let _ = ShowWindow(hWnd, SW_SHOW);
        let _ = UpdateWindow(hWnd);

        let mut current_visible = IsWindowVisible(hWnd).as_bool();

        loop {
            let enabled = ENABLED.load(Ordering::Relaxed);

            if current_visible != enabled {
                current_visible = enabled;
                if enabled {
                    let _ = ShowWindow(hWnd, SW_SHOW);
                } else {
                    let _ = ShowWindow(hWnd, SW_HIDE);
                }
            }

            if enabled {
                render(&mut g, &frag);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

unsafe fn init_d3d11(hWnd: HWND) -> windows::core::Result<Globals> {
    unsafe {
        let virt_w = GetSystemMetrics(SM_CXVIRTUALSCREEN) as u32;
        let virt_h = GetSystemMetrics(SM_CYVIRTUALSCREEN) as u32;

        let mut sd: DXGI_SWAP_CHAIN_DESC = zeroed();
        sd.BufferCount = 1;
        sd.BufferDesc = DXGI_MODE_DESC {
            Width: virt_w,
            Height: virt_h,
            RefreshRate: DXGI_RATIONAL {
                Numerator: 60,
                Denominator: 1,
            },
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ..Default::default()
        };
        sd.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
        sd.OutputWindow = hWnd;
        sd.SampleDesc = DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        };
        sd.Windowed = BOOL(1);
        sd.SwapEffect = DXGI_SWAP_EFFECT_DISCARD;

        let mut device: Option<ID3D11Device> = None;
        let mut ctx: Option<ID3D11DeviceContext> = None;
        let mut swap_chain: Option<IDXGISwapChain> = None;

        let feature_levels = [D3D_FEATURE_LEVEL_11_0];
        let mut got_level = D3D_FEATURE_LEVEL_11_0;

        D3D11CreateDeviceAndSwapChain(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            Default::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&sd),
            Some(&mut swap_chain),
            Some(&mut device),
            Some(&mut got_level),
            Some(&mut ctx),
        )?;

        let device = device.unwrap();
        let ctx = ctx.unwrap();
        let swap_chain = swap_chain.unwrap();

        let backbuf = swap_chain.GetBuffer::<ID3D11Texture2D>(0)?;

        let rtv = {
            let mut rtv: Option<ID3D11RenderTargetView> = None;
            device.CreateRenderTargetView(&backbuf, None, Some(&mut rtv))?;
            rtv.unwrap()
        };
        ctx.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);

        let vp = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: virt_w as f32,
            Height: virt_h as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        ctx.RSSetViewports(Some(&[vp]));

        let sampler = {
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MinLOD: 0.0,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            let mut s: Option<ID3D11SamplerState> = None;
            device.CreateSamplerState(&desc, Some(&mut s))?;
            s.unwrap()
        };

        let (vs, input_layout) = {
            let mut vs_blob: Option<ID3DBlob> = None;
            let mut err_blob: Option<ID3DBlob> = None;

            D3DCompile(
                VS_SRC.as_ptr() as _,
                VS_SRC.len(),
                None,
                None,
                None,
                s!("main"),
                s!("vs_5_0"),
                0,
                0,
                &mut vs_blob,
                Some(&mut err_blob),
            )?;
            let vs_blob = vs_blob.unwrap();
            let vs_blob_slice = slice::from_raw_parts(
                vs_blob.GetBufferPointer() as *const u8,
                vs_blob.GetBufferSize(),
            );

            let mut vs: Option<ID3D11VertexShader> = None;
            device.CreateVertexShader(vs_blob_slice, None, Some(&mut vs))?;
            let vs = vs.unwrap();

            let layout_desc = [
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: s!("POSITION"),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: 0,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: s!("TEXCOORD"),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: 8,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
            ];

            let mut il: Option<ID3D11InputLayout> = None;
            device.CreateInputLayout(&layout_desc, vs_blob_slice, Some(&mut il))?;
            let il = il.unwrap();
            (vs, il)
        };

        let vb = {
            let vertices = [
                SimpleVertex {
                    x: -1.0,
                    y: 1.0,
                    u: 0.0,
                    v: 0.0,
                },
                SimpleVertex {
                    x: 1.0,
                    y: 1.0,
                    u: 1.0,
                    v: 0.0,
                },
                SimpleVertex {
                    x: -1.0,
                    y: -1.0,
                    u: 0.0,
                    v: 1.0,
                },
                SimpleVertex {
                    x: 1.0,
                    y: -1.0,
                    u: 1.0,
                    v: 1.0,
                },
            ];
            let bd = D3D11_BUFFER_DESC {
                ByteWidth: size_of::<SimpleVertex>() as u32 * 4,
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
                ..Default::default()
            };
            let init = D3D11_SUBRESOURCE_DATA {
                pSysMem: vertices.as_ptr() as *const c_void,
                ..Default::default()
            };
            let mut buf: Option<ID3D11Buffer> = None;
            device.CreateBuffer(&bd, Some(&init), Some(&mut buf))?;
            buf.unwrap()
        };

        Ok(Globals {
            device,
            ctx,
            swap_chain,
            rtv,
            vs,
            input_layout,
            vb,
            sampler,
            outputs: Vec::new(),
        })
    }
}

unsafe fn compile_shader(globals: &Globals, code: &str) -> windows::core::Result<FragmentShader> {
    unsafe {
        let mut ps_blob: Option<ID3DBlob> = None;
        let mut err_blob: Option<ID3DBlob> = None;

        D3DCompile(
            code.as_ptr() as _,
            code.len(),
            None,
            None,
            None,
            s!("main"),
            s!("ps_5_0"),
            0,
            0,
            &mut ps_blob,
            Some(&mut err_blob),
        )?;
        let ps_blob = ps_blob.unwrap();
        let ps_blob_slice = slice::from_raw_parts(
            ps_blob.GetBufferPointer() as *const u8,
            ps_blob.GetBufferSize(),
        );

        let mut ps: Option<ID3D11PixelShader> = None;
        globals
            .device
            .CreatePixelShader(ps_blob_slice, None, Some(&mut ps))?;
        Ok(FragmentShader { ps: ps.unwrap() })
    }
}

unsafe fn init_duplications(g: &mut Globals) -> windows::core::Result<()> {
    unsafe {
        g.outputs.clear();

        let mut dxgi_device: Option<IDXGIDevice> = None;
        let _ = g
            .device
            .query(&IDXGIDevice::IID, std::mem::transmute(&mut dxgi_device));
        let dxgi_device = dxgi_device.unwrap();

        let adapter = dxgi_device.GetParent::<IDXGIAdapter>()?;

        let mut i = 0;
        while let Ok(output) = adapter.EnumOutputs(i) {
            let mut output1: Option<IDXGIOutput1> = None;
            let _ = output.query(&IDXGIOutput1::IID, std::mem::transmute(&mut output1));
            let output1 = output1.unwrap();

            let outdesc = output.GetDesc()?;

            let dup = output1.DuplicateOutput(&g.device)?;
            let dud = dup.GetDesc();

            let desc = D3D11_TEXTURE2D_DESC {
                Width: dud.ModeDesc.Width,
                Height: dud.ModeDesc.Height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                ..Default::default()
            };
            let mut tex: Option<ID3D11Texture2D> = None;
            g.device.CreateTexture2D(&desc, None, Some(&mut tex))?;
            let tex = tex.unwrap();

            let srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
                Format: desc.Format,
                ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_SRV {
                        MostDetailedMip: 0,
                        MipLevels: 1,
                    },
                },
            };
            let mut srv: Option<ID3D11ShaderResourceView> = None;
            g.device
                .CreateShaderResourceView(&tex, Some(&srv_desc), Some(&mut srv))?;
            let srv = srv.unwrap();

            g.outputs.push(OutputDup {
                dup,
                desktop_rect: outdesc.DesktopCoordinates,
                width: dud.ModeDesc.Width,
                height: dud.ModeDesc.Height,
                dest_tex: tex,
                dest_srv: srv,
            });

            i += 1;
        }

        if g.outputs.is_empty() {
            Err(E_FAIL.into())
        } else {
            Ok(())
        }
    }
}

unsafe fn capture_desktop_per_output(g: &mut Globals) {
    unsafe {
        if g.outputs.is_empty() {
            return;
        }
        let mut need_reinit = false;

        for od in &mut g.outputs {
            let mut frame_info: DXGI_OUTDUPL_FRAME_INFO = zeroed();
            let mut desktop_res: Option<IDXGIResource> = None;

            match od
                .dup
                .AcquireNextFrame(0, &mut frame_info, &mut desktop_res)
            {
                Ok(_) => {}
                Err(err) => {
                    let code = err.code().0 as i32;
                    if code == DXGI_ERROR_WAIT_TIMEOUT.0 {
                        continue;
                    } else if code == DXGI_ERROR_ACCESS_LOST.0 {
                        need_reinit = true;
                        continue;
                    } else {
                        continue;
                    }
                }
            };
            let desktop_res = desktop_res.unwrap();

            let mut src_tex: Option<ID3D11Texture2D> = None;
            let _ = desktop_res.query(&ID3D11Texture2D::IID, std::mem::transmute(&mut src_tex));
            let src_tex = src_tex.unwrap();

            let mut src_desc: D3D11_TEXTURE2D_DESC = Default::default();
            src_tex.GetDesc(&mut src_desc);

            let dest_fmt = DXGI_FORMAT_B8G8R8A8_UNORM;

            if src_desc.Format == dest_fmt
                && src_desc.Width == od.width
                && src_desc.Height == od.height
            {
                let box_ = D3D11_BOX {
                    left: 0,
                    top: 0,
                    front: 0,
                    right: src_desc.Width,
                    bottom: src_desc.Height,
                    back: 1,
                };
                g.ctx
                    .CopySubresourceRegion(&od.dest_tex, 0, 0, 0, 0, &src_tex, 0, Some(&box_));
            } else if (src_desc.Format == DXGI_FORMAT_R8G8B8A8_UNORM
                || src_desc.Format == DXGI_FORMAT_B8G8R8A8_UNORM)
                && src_desc.Width == od.width
                && src_desc.Height == od.height
            {
                let mut staging_desc = src_desc.clone();
                staging_desc.Usage = D3D11_USAGE_STAGING;
                staging_desc.BindFlags = 0;
                staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                staging_desc.MiscFlags = 0;
                staging_desc.SampleDesc.Count = 1;

                let mut staging: Option<ID3D11Texture2D> = None;
                if g.device
                    .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                    .is_ok()
                {
                    let staging = staging.unwrap();
                    g.ctx.CopyResource(&staging, &src_tex);

                    let mut mapped: D3D11_MAPPED_SUBRESOURCE = zeroed();
                    if g.ctx
                        .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                        .is_ok()
                    {
                        let row_bytes = od.width as usize * 4;
                        let total = row_bytes * od.height as usize;
                        let mut full = vec![0u8; total];

                        let src_is_rgba = src_desc.Format == DXGI_FORMAT_R8G8B8A8_UNORM;
                        for y in 0..od.height {
                            let srow = (mapped.pData as *const u8)
                                .add(y as usize * mapped.RowPitch as usize);
                            let drow = full.as_mut_ptr().add(y as usize * row_bytes);
                            if src_is_rgba {
                                for x in 0..od.width {
                                    let r = *srow.add(4 * x as usize + 0);
                                    let g = *srow.add(4 * x as usize + 1);
                                    let b = *srow.add(4 * x as usize + 2);
                                    let a = *srow.add(4 * x as usize + 3);
                                    *drow.add(4 * x as usize + 0) = b;
                                    *drow.add(4 * x as usize + 1) = g;
                                    *drow.add(4 * x as usize + 2) = r;
                                    *drow.add(4 * x as usize + 3) = a;
                                }
                            } else {
                                std::ptr::copy_nonoverlapping(srow, drow, row_bytes);
                            }
                        }
                        g.ctx.Unmap(&staging, 0);

                        g.ctx.UpdateSubresource(
                            &od.dest_tex,
                            0,
                            None,
                            full.as_ptr() as *const c_void,
                            row_bytes as u32,
                            0,
                        );
                    }
                }
            } else {
            }

            let _ = od.dup.ReleaseFrame();
        }

        if need_reinit {
            let _ = init_duplications(g);
        }
    }
}

unsafe fn render(g: &mut Globals, frag: &FragmentShader) {
    unsafe {
        capture_desktop_per_output(g);

        let clear = [0.0f32, 0.0, 0.0, 0.0];
        g.ctx.ClearRenderTargetView(&g.rtv, &clear);

        g.ctx.VSSetShader(&g.vs, None);
        g.ctx.PSSetShader(&frag.ps, None);
        g.ctx.PSSetSamplers(0, Some(&[Some(g.sampler.clone())]));
        g.ctx.IASetInputLayout(&g.input_layout);

        let stride = size_of::<SimpleVertex>() as u32;
        let offset = 0u32;
        g.ctx.IASetVertexBuffers(
            0,
            1,
            Some(&Some(g.vb.clone())),
            Some(&stride),
            Some(&offset),
        );
        g.ctx
            .IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);

        let virt_left = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let virt_top = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let virt_w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let virt_h = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        for od in &g.outputs {
            let vp = D3D11_VIEWPORT {
                TopLeftX: (od.desktop_rect.left - virt_left) as f32,
                TopLeftY: (od.desktop_rect.top - virt_top) as f32,
                Width: od.width as f32,
                Height: od.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            g.ctx.RSSetViewports(Some(&[vp]));

            g.ctx
                .PSSetShaderResources(0, Some(&[Some(od.dest_srv.clone())]));

            g.ctx.Draw(4, 0);

            g.ctx.PSSetShaderResources(0, Some(&[None]));
        }

        let full = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: virt_w as f32,
            Height: virt_h as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        g.ctx.RSSetViewports(Some(&[full]));

        let _ = g.swap_chain.Present(0, DXGI_PRESENT::default());
    }
}
