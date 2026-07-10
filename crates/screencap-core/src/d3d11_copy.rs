use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};

use crate::types::{ErrorInfo, ImageBuffer, Rect};

fn win_error(message: &str, where_: &str, e: windows::core::Error) -> ErrorInfo {
    ErrorInfo::with_hresult(message, where_, e.code().0 as u32)
}

pub fn create_d3d11_device(where_: &str) -> Result<(ID3D11Device, ID3D11DeviceContext), ErrorInfo> {
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(|e| win_error("D3D11CreateDevice failed", where_, e))?;
    let device =
        device.ok_or_else(|| ErrorInfo::new("D3D11CreateDevice returned no device", where_))?;
    let context =
        context.ok_or_else(|| ErrorInfo::new("D3D11CreateDevice returned no context", where_))?;
    Ok((device, context))
}

struct MappedTexture<'a> {
    context: &'a ID3D11DeviceContext,
    texture: &'a ID3D11Texture2D,
    map: D3D11_MAPPED_SUBRESOURCE,
}

impl<'a> MappedTexture<'a> {
    fn new(
        context: &'a ID3D11DeviceContext,
        texture: &'a ID3D11Texture2D,
        where_: &str,
    ) -> Result<Self, ErrorInfo> {
        let mut map = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { context.Map(texture, 0, D3D11_MAP_READ, 0, Some(&mut map)) }
            .map_err(|e| win_error("Map staging failed", where_, e))?;
        Ok(Self {
            context,
            texture,
            map,
        })
    }

    fn row_pitch(&self) -> usize {
        self.map.RowPitch as usize
    }

    fn data(&self) -> *const u8 {
        self.map.pData as *const u8
    }
}

impl Drop for MappedTexture<'_> {
    fn drop(&mut self) {
        unsafe {
            self.context.Unmap(self.texture, 0);
        }
    }
}

pub(crate) fn copy_texture_to_image(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    mut desc: D3D11_TEXTURE2D_DESC,
    rect: Rect,
    copy_to_staging: impl FnOnce(&ID3D11Texture2D),
    where_: &'static str,
) -> Result<ImageBuffer, ErrorInfo> {
    let width = rect.width();
    let height = rect.height();
    desc.Width = width as u32;
    desc.Height = height as u32;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;
    desc.Usage = D3D11_USAGE_STAGING;

    let mut staging: Option<ID3D11Texture2D> = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)) }
        .map_err(|e| win_error("CreateTexture2D staging failed", where_, e))?;
    let staging =
        staging.ok_or_else(|| ErrorInfo::new("CreateTexture2D staging failed", where_))?;

    copy_to_staging(&staging);

    let mapped = MappedTexture::new(context, &staging, where_)?;

    let row_pitch = width * 4;
    let mut bgra = Vec::with_capacity((row_pitch as usize) * (height as usize));
    for y in 0..height {
        let src = unsafe {
            std::slice::from_raw_parts(
                mapped.data().add((y as usize) * mapped.row_pitch()),
                row_pitch as usize,
            )
        };
        bgra.extend_from_slice(src);
    }

    Ok(ImageBuffer {
        width,
        height,
        row_pitch,
        origin_x: rect.left,
        origin_y: rect.top,
        bgra,
    })
}
