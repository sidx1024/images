//! Background decoding with WIC plus metadata via the shell property system.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};

use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::GlobalFree;
use windows::Win32::Foundation::{
    E_FAIL, E_OUTOFMEMORY, GENERIC_READ, HGLOBAL, HWND, LPARAM, PROPERTYKEY, SYSTEMTIME, WPARAM,
};
use windows::Win32::Globalization::{
    GetDateFormatEx, GetTimeFormatEx, ENUM_DATE_FORMATS_FLAGS, TIME_NOSECONDS,
};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::Storage::EnhancedStorage::*;
use windows::Win32::System::Com::StructuredStorage::{CreateStreamOnHGlobal, GetHGlobalFromStream};
use windows::Win32::System::Com::StructuredStorage::{
    PropVariantClear, PropVariantToFileTime, PropVariantToUInt16, PROPVARIANT,
};
use windows::Win32::System::Com::STREAM_SEEK_END;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTimeEx};
use windows::Win32::System::Variant::{PSTF_UTC, VT_EMPTY};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IPropertyStore, PSFormatForDisplayAlloc, SHGetPropertyStoreFromParsingName, GPS_BESTEFFORT,
    PDFF_DEFAULT,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::wide;

pub const WM_APP_LOADED: u32 = WM_APP + 1;

pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// Size of the image as stored (after orientation), may exceed width/height if downscaled for the GPU.
    pub full_width: u32,
    pub full_height: u32,
    /// Premultiplied BGRA, stride = width * 4.
    pub pixels: Vec<u8>,
    pub dpi: f64,
    pub bits_per_pixel: u32,
    pub file_bytes: u64,
}

/// Movable global memory destined for the clipboard; freed on drop unless handed over.
pub struct GlobalBuf(HGLOBAL);

// The handle is plain memory owned by this value; it's only touched by one thread at a time.
unsafe impl Send for GlobalBuf {}

impl GlobalBuf {
    fn new(len: usize) -> windows::core::Result<Self> {
        unsafe { GlobalAlloc(GMEM_MOVEABLE, len.max(1)).map(GlobalBuf) }
    }

    /// Fill via `f`, which receives the locked bytes.
    fn fill(&self, len: usize, f: impl FnOnce(&mut [u8])) -> windows::core::Result<()> {
        unsafe {
            let ptr = GlobalLock(self.0) as *mut u8;
            if ptr.is_null() {
                return Err(windows::core::Error::from(E_OUTOFMEMORY));
            }
            f(std::slice::from_raw_parts_mut(ptr, len));
            let _ = GlobalUnlock(self.0);
        }
        Ok(())
    }

    /// Give up ownership (after SetClipboardData succeeds, the clipboard owns the memory).
    pub fn into_handle(self) -> HGLOBAL {
        let h = self.0;
        std::mem::forget(self);
        h
    }
}

impl Drop for GlobalBuf {
    fn drop(&mut self) {
        unsafe {
            let _ = GlobalFree(Some(self.0));
        }
    }
}

/// What Ctrl+C puts on the clipboard, prepared off the UI thread.
pub struct ClipData {
    /// "PNG", only for images with transparency (keeps alpha in apps that read it). Published first.
    pub png: Option<GlobalBuf>,
    /// CF_DIB: BITMAPINFOHEADER + bottom-up 32-bit rows, flattened onto white. Readable by every app.
    pub dib: GlobalBuf,
}

#[derive(Default, Clone)]
pub struct ImageInfo {
    /// Long local date and time on two lines, e.g. "September 30, 2026\n7:46 PM".
    pub date_taken: Option<String>,
    pub date_modified: Option<String>,
    pub camera: Option<String>,
    pub f_number: Option<String>,
    pub exposure: Option<String>,
    pub focal_length: Option<String>,
    pub iso: Option<String>,
    pub lens: Option<String>,
    pub flash: Option<String>,
    pub author: Option<String>,
    pub program: Option<String>,
}

pub enum Msg {
    Image {
        path: PathBuf,
        result: Result<Decoded, String>,
    },
    Info {
        path: PathBuf,
        info: ImageInfo,
    },
    Folder {
        generation: u64,
        files: Vec<PathBuf>,
    },
    /// Result of copy request `id`; the error is a user-facing message.
    Clipboard {
        id: u64,
        result: Result<ClipData, String>,
    },
}

/// Posts to the window once its handle is known; results sent before that are drained at startup.
#[derive(Clone)]
pub struct Notifier {
    pub tx: Sender<Msg>,
    pub hwnd: Arc<AtomicIsize>,
}

impl Notifier {
    pub fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        let h = self.hwnd.load(Ordering::Acquire);
        if h != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as _)), WM_APP_LOADED, WPARAM(0), LPARAM(0));
            }
        }
    }
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<PathBuf>,
    in_flight: HashSet<PathBuf>,
}

pub struct Loader {
    shared: Arc<(Mutex<Queue>, Condvar)>,
    info: Arc<(Mutex<Option<PathBuf>>, Condvar)>,
    pub max_dim: Arc<AtomicU32>,
}

/// Upper bound on decoded pixels for viewing (100 MP = 400 MB of BGRA) so one huge file can't exhaust memory.
const MAX_PIXELS: f64 = 100_000_000.0;
/// Clipboard copies are full resolution up to this size; larger images are refused rather than shrunk.
const MAX_CLIPBOARD_PIXELS: f64 = 150_000_000.0;
const TOO_LARGE: &str = "This image is too large to copy.";

#[derive(Clone, Copy)]
enum PixelLimit {
    /// Scale down to fit within this many pixels.
    Downscale(f64),
    /// Fail if the image has more pixels than this.
    Refuse(f64),
}

impl Loader {
    pub fn new(notifier: Notifier, threads: usize) -> Self {
        let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let max_dim = Arc::new(AtomicU32::new(16384));
        for _ in 0..threads {
            let shared = shared.clone();
            let notifier = notifier.clone();
            let max_dim = max_dim.clone();
            std::thread::spawn(move || worker(shared, notifier, max_dim));
        }
        // Metadata gets its own thread so slow shell property handlers never delay pixels.
        let info = Arc::new((Mutex::new(None), Condvar::new()));
        let slot = info.clone();
        std::thread::spawn(move || info_worker(slot, notifier));
        Loader {
            shared,
            info,
            max_dim,
        }
    }

    /// Load metadata for `path`, replacing any request not yet started.
    pub fn request_info(&self, path: PathBuf) {
        let (lock, cv) = &*self.info;
        *lock.lock().unwrap() = Some(path);
        cv.notify_one();
    }

    /// Replace the queue with `paths` (in priority order), skipping anything already decoding.
    pub fn set_jobs(&self, paths: Vec<PathBuf>) {
        let (lock, cv) = &*self.shared;
        let mut q = lock.lock().unwrap();
        q.jobs = paths
            .into_iter()
            .filter(|p| !q.in_flight.contains(p))
            .collect();
        cv.notify_all();
    }
}

fn worker(shared: Arc<(Mutex<Queue>, Condvar)>, notifier: Notifier, max_dim: Arc<AtomicU32>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let factory: Option<IWICImagingFactory> =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok() };
    loop {
        let path = {
            let (lock, cv) = &*shared;
            let mut q = lock.lock().unwrap();
            loop {
                if let Some(p) = q.jobs.pop_front() {
                    q.in_flight.insert(p.clone());
                    break p;
                }
                q = cv.wait(q).unwrap();
            }
        };
        let result = match &factory {
            Some(f) => decode_file(f, &path, max_dim.load(Ordering::Relaxed)),
            None => Err("Windows Imaging Component is unavailable".into()),
        };
        shared.0.lock().unwrap().in_flight.remove(&path);
        notifier.send(Msg::Image { path, result });
    }
}

fn info_worker(slot: Arc<(Mutex<Option<PathBuf>>, Condvar)>, notifier: Notifier) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    loop {
        let path = {
            let (lock, cv) = &*slot;
            let mut p = lock.lock().unwrap();
            loop {
                if let Some(path) = p.take() {
                    break path;
                }
                p = cv.wait(p).unwrap();
            }
        };
        let info = read_info(&path);
        notifier.send(Msg::Info { path, info });
    }
}

/// Only a failure in color management is worth a second, unmanaged attempt; out-of-memory (including
/// "too large") would just fail again after repeating all the work.
fn retry_without_color(e: &windows::core::Error) -> bool {
    e.code() != E_OUTOFMEMORY
}

fn decode_file(factory: &IWICImagingFactory, path: &Path, max_dim: u32) -> Result<Decoded, String> {
    // Color management is best-effort: if the profile transform fails, show unmanaged pixels.
    let pbgra = &GUID_WICPixelFormat32bppPBGRA;
    let limit = PixelLimit::Downscale(MAX_PIXELS);
    unsafe {
        decode_inner(factory, path, max_dim, true, pbgra, limit).or_else(|e| {
            if retry_without_color(&e) {
                decode_inner(factory, path, max_dim, false, pbgra, limit)
            } else {
                Err(e)
            }
        })
    }
    .map_err(|e| {
        let msg = e.message();
        if msg.is_empty() {
            format!("{:?}", e.code())
        } else {
            msg
        }
    })
}

unsafe fn decode_inner(
    factory: &IWICImagingFactory,
    path: &Path,
    max_dim: u32,
    manage_color: bool,
    format: &windows::core::GUID,
    limit: PixelLimit,
) -> windows::core::Result<Decoded> {
    let wpath = wide(path);
    let decoder = factory.CreateDecoderFromFilename(
        PCWSTR(wpath.as_ptr()),
        None,
        GENERIC_READ,
        WICDecodeMetadataCacheOnDemand,
    )?;
    let frame = decoder.GetFrame(0)?;
    let (mut w, mut h) = (0u32, 0u32);
    frame.GetSize(&mut w, &mut h)?;
    let orientation = read_orientation(&frame);
    let (mut dpi, mut dpi_y) = (0f64, 0f64);
    let _ = frame.GetResolution(&mut dpi, &mut dpi_y);
    let bits_per_pixel = frame
        .GetPixelFormat()
        .and_then(|g| factory.CreateComponentInfo(&g))
        .and_then(|i| i.cast::<IWICPixelFormatInfo>())
        .and_then(|i| i.GetBitsPerPixel())
        .unwrap_or(0);
    let file_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let mut source: IWICBitmapSource = frame.cast()?;

    let pixels = w as f64 * h as f64;
    let max_pixels = match limit {
        PixelLimit::Downscale(max) => max,
        PixelLimit::Refuse(max) if pixels > max => {
            return Err(windows::core::Error::new(E_OUTOFMEMORY, TOO_LARGE))
        }
        PixelLimit::Refuse(_) => f64::INFINITY,
    };
    // Downscale only if the GPU can't hold the full image in one texture, or it exceeds the memory cap.
    let k = (max_dim as f64 / w.max(h) as f64).min((max_pixels / pixels).sqrt());
    if k < 1.0 {
        let (nw, nh) = (
            ((w as f64 * k) as u32).max(1),
            ((h as f64 * k) as u32).max(1),
        );
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&source, nw, nh, WICBitmapInterpolationModeFant)?;
        source = scaler.cast()?;
    }

    let transform = match orientation {
        2 => WICBitmapTransformFlipHorizontal,
        3 => WICBitmapTransformRotate180,
        4 => WICBitmapTransformFlipVertical,
        5 => WICBitmapTransformOptions(
            WICBitmapTransformRotate90.0 | WICBitmapTransformFlipHorizontal.0,
        ),
        6 => WICBitmapTransformRotate90,
        7 => WICBitmapTransformOptions(
            WICBitmapTransformRotate270.0 | WICBitmapTransformFlipHorizontal.0,
        ),
        8 => WICBitmapTransformRotate270,
        _ => WICBitmapTransformRotate0,
    };
    let swapped = matches!(orientation, 5..=8);

    if manage_color {
        if let Some(managed) = to_srgb(factory, &frame, &source)? {
            source = managed;
        }
    }

    let conv = factory.CreateFormatConverter()?;
    conv.Initialize(
        &source,
        format,
        WICBitmapDitherTypeNone,
        None,
        0.0,
        WICBitmapPaletteTypeMedianCut,
    )?;
    let mut output: IWICBitmapSource = conv.cast()?;
    if transform != WICBitmapTransformRotate0 {
        // Rotate an in-memory copy. Fed straight from a decoder, the flip-rotator pulls pixels in
        // column order and the JPEG decoder re-decodes the image for every request, which turns one
        // portrait photo into minutes of work and stalls the worker.
        let decoded = factory.CreateBitmapFromSource(&output, WICBitmapCacheOnLoad)?;
        let rot = factory.CreateBitmapFlipRotator()?;
        rot.Initialize(&decoded, transform)?;
        output = rot.cast()?;
    }
    let (mut cw, mut ch) = (0u32, 0u32);
    output.GetSize(&mut cw, &mut ch)?;
    let stride = cw
        .checked_mul(4)
        .ok_or(windows::core::Error::from(E_OUTOFMEMORY))?;
    let len = (stride as usize)
        .checked_mul(ch as usize)
        .ok_or(windows::core::Error::from(E_OUTOFMEMORY))?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(len)
        .map_err(|_| windows::core::Error::from(E_OUTOFMEMORY))?;
    pixels.resize(len, 0);
    output.CopyPixels(std::ptr::null(), stride, &mut pixels)?;

    let (full_width, full_height) = if swapped { (h, w) } else { (w, h) };
    Ok(Decoded {
        width: cw,
        height: ch,
        full_width,
        full_height,
        pixels,
        dpi,
        bits_per_pixel,
        file_bytes,
    })
}

/// Prepare a full-resolution, upright, sRGB copy of `path` for the clipboard on a background thread.
pub fn copy_async(id: u64, path: PathBuf, notifier: Notifier) {
    std::thread::spawn(move || {
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let result = unsafe { clipboard_data(&path) }.map_err(|e| {
            if e.message() == TOO_LARGE {
                TOO_LARGE.to_string()
            } else {
                "Couldn't copy the image.".to_string()
            }
        });
        if com {
            // All WIC objects were dropped inside clipboard_data.
            unsafe { CoUninitialize() };
        }
        notifier.send(Msg::Clipboard { id, result });
    });
}

unsafe fn clipboard_data(path: &Path) -> windows::core::Result<ClipData> {
    let factory: IWICImagingFactory =
        CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
    // Straight (not premultiplied) alpha is what clipboard consumers expect; no GPU size limit applies.
    let bgra = &GUID_WICPixelFormat32bppBGRA;
    let limit = PixelLimit::Refuse(MAX_CLIPBOARD_PIXELS);
    let d = decode_inner(&factory, path, u32::MAX, true, bgra, limit).or_else(|e| {
        if retry_without_color(&e) {
            decode_inner(&factory, path, u32::MAX, false, bgra, limit)
        } else {
            Err(e)
        }
    })?;
    // Check the pixels rather than the format: indexed GIF/PNG palettes can carry transparency too.
    let transparent = d.pixels.chunks_exact(4).any(|px| px[3] != 255);
    let png = if transparent {
        Some(encode_png(&factory, d.width, d.height, &d.pixels)?)
    } else {
        None
    };
    let dib = make_dib(d.width, d.height, &d.pixels, transparent)?;
    Ok(ClipData { png, dib })
}

/// CF_DIB payload: a 40-byte BITMAPINFOHEADER followed by bottom-up rows. BI_RGB has no alpha, so
/// transparent images are composited onto white instead of exposing whatever color hides under alpha 0.
fn make_dib(
    width: u32,
    height: u32,
    pixels: &[u8],
    flatten: bool,
) -> windows::core::Result<GlobalBuf> {
    let stride = width as usize * 4;
    let size = stride * height as usize;
    let buf = GlobalBuf::new(40 + size)?;
    buf.fill(40 + size, |out| {
        let (header, body) = out.split_at_mut(40);
        let mut h = Vec::with_capacity(40);
        h.extend(40u32.to_le_bytes()); // biSize
        h.extend((width as i32).to_le_bytes());
        h.extend((height as i32).to_le_bytes()); // positive = bottom-up, the most compatible layout
        h.extend(1u16.to_le_bytes()); // biPlanes
        h.extend(32u16.to_le_bytes()); // biBitCount
        h.extend(0u32.to_le_bytes()); // BI_RGB
        h.extend((size as u32).to_le_bytes());
        h.extend([0u8; 16]); // resolution, colors used, colors important
        header.copy_from_slice(&h);
        for (dst, src) in body
            .chunks_exact_mut(stride)
            .zip(pixels.chunks_exact(stride).rev())
        {
            dst.copy_from_slice(src);
            if flatten {
                for px in dst.chunks_exact_mut(4) {
                    let a = px[3] as u32;
                    for c in &mut px[..3] {
                        *c = ((*c as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
                    }
                    px[3] = 255;
                }
            }
        }
    })?;
    Ok(buf)
}

unsafe fn encode_png(
    factory: &IWICImagingFactory,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> windows::core::Result<GlobalBuf> {
    let stream = CreateStreamOnHGlobal(HGLOBAL::default(), true)?;
    let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
    encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
    let (mut frame, mut props) = (None, None);
    encoder.CreateNewFrame(&mut frame, &mut props)?;
    let frame = frame.ok_or(windows::core::Error::from(E_FAIL))?;
    frame.Initialize(props.as_ref())?;
    frame.SetSize(width, height)?;
    let mut format = GUID_WICPixelFormat32bppBGRA;
    frame.SetPixelFormat(&mut format)?;
    frame.WritePixels(height, width * 4, pixels)?;
    frame.Commit()?;
    encoder.Commit()?;
    // The stream's HGLOBAL can be larger than what was written; the stream's end is the real size.
    let mut len = 0u64;
    stream.Seek(0, STREAM_SEEK_END, Some(&mut len))?;
    let src = GetHGlobalFromStream(&stream)?;
    let ptr = GlobalLock(src) as *const u8;
    if ptr.is_null() {
        return Err(windows::core::Error::from(E_FAIL));
    }
    let len = len as usize;
    let copy = || -> windows::core::Result<GlobalBuf> {
        let out = GlobalBuf::new(len)?;
        out.fill(len, |dst| {
            dst.copy_from_slice(std::slice::from_raw_parts(ptr, len))
        })?;
        Ok(out)
    };
    let result = copy();
    let _ = GlobalUnlock(src);
    result
}

/// Wrap `source` in a transform from the frame's embedded ICC profile to sRGB. `None` if there's no profile.
unsafe fn to_srgb(
    factory: &IWICImagingFactory,
    frame: &IWICBitmapFrameDecode,
    source: &IWICBitmapSource,
) -> windows::core::Result<Option<IWICBitmapSource>> {
    let mut count = 0u32;
    if frame.GetColorContexts(&mut [], &mut count).is_err() || count == 0 {
        return Ok(None);
    }
    let mut contexts: Vec<Option<IWICColorContext>> = (0..count)
        .map(|_| factory.CreateColorContext().ok())
        .collect();
    if contexts.iter().any(|c| c.is_none())
        || frame.GetColorContexts(&mut contexts, &mut count).is_err()
    {
        return Ok(None);
    }
    let Some(profile) = contexts
        .into_iter()
        .flatten()
        .find(|c| c.GetType().ok() == Some(WICColorContextProfile))
    else {
        return Ok(None);
    };
    let srgb = factory.CreateColorContext()?;
    srgb.InitializeFromExifColorSpace(1)?;
    // The transformer wants a plain 32bpp BGRA input.
    let bgra = factory.CreateFormatConverter()?;
    bgra.Initialize(
        source,
        &GUID_WICPixelFormat32bppBGRA,
        WICBitmapDitherTypeNone,
        None,
        0.0,
        WICBitmapPaletteTypeMedianCut,
    )?;
    let xform = factory.CreateColorTransformer()?;
    xform.Initialize(&bgra, &profile, &srgb, &GUID_WICPixelFormat32bppBGRA)?;
    Ok(Some(xform.cast()?))
}

/// EXIF orientation (1..=8), defaulting to 1.
unsafe fn read_orientation(frame: &IWICBitmapFrameDecode) -> u16 {
    let Ok(reader) = frame.GetMetadataQueryReader() else {
        return 1;
    };
    for name in [
        "System.Photo.Orientation",
        "/app1/ifd/{ushort=274}",
        "/ifd/{ushort=274}",
    ] {
        let wname = wide(name);
        let mut pv = PROPVARIANT::default();
        if reader
            .GetMetadataByName(PCWSTR(wname.as_ptr()), &mut pv)
            .is_ok()
        {
            let v = PropVariantToUInt16(&pv).unwrap_or(1);
            let _ = PropVariantClear(&mut pv);
            if (1..=8).contains(&v) {
                return v;
            }
        }
    }
    1
}

fn read_info(path: &Path) -> ImageInfo {
    unsafe {
        let wpath = wide(path);
        let Ok(store) = SHGetPropertyStoreFromParsingName::<_, _, IPropertyStore>(
            PCWSTR(wpath.as_ptr()),
            None,
            GPS_BESTEFFORT,
        ) else {
            return ImageInfo::default();
        };
        let get = |key: &PROPERTYKEY| prop_display(&store, key);

        let make = get(&PKEY_Photo_CameraManufacturer);
        let model = get(&PKEY_Photo_CameraModel);
        let camera = match (make, model) {
            (Some(make), Some(model)) if model.to_lowercase().starts_with(&make.to_lowercase()) => {
                Some(model)
            }
            (Some(make), Some(model)) => Some(format!("{make} {model}")),
            (a, b) => a.or(b),
        };

        ImageInfo {
            date_taken: prop_date(&store, &PKEY_Photo_DateTaken),
            date_modified: prop_date(&store, &PKEY_DateModified),
            camera,
            f_number: get(&PKEY_Photo_FNumber),
            exposure: get(&PKEY_Photo_ExposureTime),
            focal_length: get(&PKEY_Photo_FocalLength),
            iso: get(&PKEY_Photo_ISOSpeed),
            lens: get(&PKEY_Photo_LensModel),
            flash: get(&PKEY_Photo_Flash),
            author: get(&PKEY_Author),
            program: get(&PKEY_SoftwareUsed),
        }
    }
}

/// A date property as local long date + time, like Photos ("September 30, 2026" / "7:46 PM").
unsafe fn prop_date(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let mut pv = store.GetValue(key).ok()?;
    if pv.Anonymous.Anonymous.vt == VT_EMPTY {
        return None;
    }
    let ft = PropVariantToFileTime(&pv, PSTF_UTC);
    let _ = PropVariantClear(&mut pv);
    let ft = ft.ok()?;
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    FileTimeToSystemTime(&ft, &mut utc).ok()?;
    SystemTimeToTzSpecificLocalTimeEx(None, &utc, &mut local).ok()?;
    let mut date = [0u16; 128];
    let n = GetDateFormatEx(
        PCWSTR::null(),
        ENUM_DATE_FORMATS_FLAGS(0),
        Some(&local),
        w!("MMMM d, yyyy"),
        Some(&mut date),
        PCWSTR::null(),
    );
    let mut time = [0u16; 64];
    let m = GetTimeFormatEx(
        PCWSTR::null(),
        TIME_NOSECONDS,
        Some(&local),
        PCWSTR::null(),
        Some(&mut time),
    );
    if n <= 1 || m <= 1 {
        return None;
    }
    let date = String::from_utf16_lossy(&date[..n as usize - 1]);
    let time = String::from_utf16_lossy(&time[..m as usize - 1]);
    Some(format!("{date}\n{time}"))
}

/// A property formatted the way Explorer shows it ("f/1.8", "1/120 sec.", "3.2 MB").
unsafe fn prop_display(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let mut pv = store.GetValue(key).ok()?;
    if pv.Anonymous.Anonymous.vt == VT_EMPTY {
        return None;
    }
    let formatted = PSFormatForDisplayAlloc(key, &pv, PDFF_DEFAULT);
    let _ = PropVariantClear(&mut pv);
    let pwstr = formatted.ok()?;
    let s = pwstr.to_string().ok();
    CoTaskMemFree(Some(pwstr.0 as _));
    // Strip the bidi marks Windows inserts into dates.
    let s: String = s?
        .chars()
        .filter(|c| !matches!(c, '\u{200e}' | '\u{200f}'))
        .collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

pub fn format_size(bytes: u64) -> String {
    let b = bytes as f64;
    match bytes {
        0..=1023 => format!("{bytes} bytes"),
        1024..=1_048_575 => format!("{:.0} KB", b / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", b / 1_048_576.0),
        _ => format!("{:.2} GB", b / 1_073_741_824.0),
    }
}
