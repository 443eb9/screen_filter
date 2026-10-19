use std::slice;

use windows::{
    Win32::{
        Foundation::E_FAIL,
        Graphics::{
            Direct3D::{Fxc::*, *},
            Direct3D11::*,
            Dxgi::{Common::*, IDXGIAdapter},
        },
    },
    core::{Error, Result, s},
};

const VERTEX_SHADER: &str = include_str!("shaders/fullscreen.hlsl");

pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    parameters: ID3D11Buffer,
    sampler: ID3D11SamplerState,
}

impl Gpu {
    pub fn new(
        adapter: Option<&IDXGIAdapter>,
        driver: D3D_DRIVER_TYPE,
        fragment: &str,
    ) -> Result<Self> {
        unsafe {
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                adapter,
                driver,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let device = device.expect("Successful D3D11 device creation returns a device");
            let context = context.expect("Successful D3D11 device creation returns a context");
            let vertex_code = Self::compile(VERTEX_SHADER, s!("vs_5_0"))?;
            let pixel_code = Self::compile(fragment, s!("ps_5_0"))?;
            let mut vertex = None;
            device.CreateVertexShader(&vertex_code, None, Some(&mut vertex))?;
            let mut pixel = None;
            device.CreatePixelShader(&pixel_code, None, Some(&mut pixel))?;
            let mut parameters = None;
            device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: 16,
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut parameters),
            )?;
            let mut sampler = None;
            device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                    Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
                    AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                    MaxLOD: f32::MAX,
                    ..Default::default()
                },
                Some(&mut sampler),
            )?;
            Ok(Self {
                device,
                context,
                vertex: vertex.expect("Shader creation returns a shader"),
                pixel: pixel.expect("Shader creation returns a shader"),
                parameters: parameters.expect("Buffer creation returns a buffer"),
                sampler: sampler.expect("Sampler creation returns a sampler"),
            })
        }
    }

    fn compile(source: &str, target: windows::core::PCSTR) -> Result<Vec<u8>> {
        unsafe {
            let mut code = None;
            let mut diagnostics = None;
            let result = D3DCompile(
                source.as_ptr().cast(),
                source.len(),
                None,
                None,
                None,
                s!("main"),
                target,
                D3DCOMPILE_IEEE_STRICTNESS,
                0,
                &mut code,
                Some(&mut diagnostics),
            );
            if let Err(error) = result {
                let message = diagnostics
                    .map(|blob: ID3DBlob| {
                        String::from_utf8_lossy(slice::from_raw_parts(
                            blob.GetBufferPointer().cast(),
                            blob.GetBufferSize(),
                        ))
                        .into_owned()
                    })
                    .unwrap_or_else(|| error.to_string());
                return Err(Error::new(E_FAIL, format!("Shader compilation: {message}")));
            }
            let code = code.expect("Successful compilation returns shader bytecode");
            Ok(
                slice::from_raw_parts(code.GetBufferPointer().cast(), code.GetBufferSize())
                    .to_vec(),
            )
        }
    }

    pub fn texture(&self, width: u32, height: u32, format: DXGI_FORMAT) -> Result<Texture> {
        unsafe {
            let mut texture = None;
            self.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: width,
                    Height: height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET).0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut texture),
            )?;
            let raw = texture.expect("Texture creation returns a texture");
            let mut view = None;
            self.device
                .CreateShaderResourceView(&raw, None, Some(&mut view))?;
            Ok(Texture {
                raw,
                view: view.expect("View creation returns a view"),
            })
        }
    }

    pub fn draw(
        &self,
        input: &Texture,
        output: &ID3D11Texture2D,
        rotation: DXGI_MODE_ROTATION,
    ) -> Result<()> {
        let rotation = match rotation {
            DXGI_MODE_ROTATION_IDENTITY | DXGI_MODE_ROTATION_UNSPECIFIED => 0u32,
            DXGI_MODE_ROTATION_ROTATE90 => 1,
            DXGI_MODE_ROTATION_ROTATE180 => 2,
            DXGI_MODE_ROTATION_ROTATE270 => 3,
            _ => return Err(Error::new(E_FAIL, "Display rotation is unsupported")),
        };
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            output.GetDesc(&mut desc);
            let mut target = None;
            self.device
                .CreateRenderTargetView(output, None, Some(&mut target))?;
            self.context.OMSetRenderTargets(Some(&[target]), None);
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                Width: desc.Width as f32,
                Height: desc.Height as f32,
                MaxDepth: 1.0,
                ..Default::default()
            }]));
            let data = [rotation, desc.Width, desc.Height, 0];
            self.context
                .UpdateSubresource(&self.parameters, 0, None, data.as_ptr().cast(), 0, 0);
            self.context
                .VSSetConstantBuffers(0, Some(&[Some(self.parameters.clone())]));
            self.context.VSSetShader(&self.vertex, None);
            self.context.PSSetShader(&self.pixel, None);
            self.context
                .PSSetShaderResources(0, Some(&[Some(input.view.clone())]));
            self.context
                .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context.IASetInputLayout(None);
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.Draw(3, 0);
            self.context.PSSetShaderResources(0, Some(&[None]));
            self.context.OMSetRenderTargets(None, None);
        }
        Ok(())
    }
}

pub struct Texture {
    pub raw: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            self.context.ClearState();
            self.context.Flush();
        }
    }
}
