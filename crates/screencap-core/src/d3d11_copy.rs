use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};

use crate::types::{ErrorInfo, ImageBuffer};

fn win_error(message: &str, where_: &str, e: windows::core::Error) -> ErrorInfo {
    ErrorInfo::with_hresult(message, where_, e.code().0 as u32)
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
    width: i32,
    height: i32,
    origin_x: i32,
    origin_y: i32,
    copy_to_staging: impl FnOnce(&ID3D11Texture2D),
    where_: &'static str,
) -> Result<ImageBuffer, ErrorInfo> {
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
    let mut bgra = vec![0u8; (row_pitch as usize) * (height as usize)];
    unsafe {
        for y in 0..height {
            let src = mapped.data().add((y as usize) * mapped.row_pitch());
            let dst = bgra.as_mut_ptr().add((y as usize) * (row_pitch as usize));
            std::ptr::copy_nonoverlapping(src, dst, row_pitch as usize);
        }
    }

    Ok(ImageBuffer {
        width,
        height,
        row_pitch,
        origin_x,
        origin_y,
        bgra,
    })
}
