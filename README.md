# images

Images app for Windows — a fast, minimal photo viewer that looks and feels like the Windows 11 Photos app, without the wait.

Native Win32 + Direct2D + WIC in Rust: a ~480 KB exe that opens a photo in about 100 ms.

<img src="assets/images.png" width="96" alt="Images app icon">

## Features

- **Viewing**: any format Windows has a codec for (JPEG, PNG, GIF, WebP, HEIC/AVIF, camera RAW, …), EXIF orientation, embedded color profiles converted to sRGB.
- **Pan & zoom**: wheel zoom at the cursor, drag to pan, double-click to toggle fit/100%, smooth eased zoom, zoom slider and presets, persistent *Fit to window* mode.
- **Navigation**: ←/→ through the folder in Explorer's order (including an open Explorer window's sort), with neighbors prefetched for instant switching.
- **Info drawer** (`I`): dimensions, size, dpi, bit depth, date taken, camera, lens, exposure, and a link that reveals the file in Explorer.
- **Photos-style chrome**: custom title bar with Snap Layouts, status bar, light/dark/system theme, high contrast, text scaling, touch gestures.
- **Actions**: Ctrl+C copies the image, Delete moves it to the Recycle Bin (with optional confirmation), F11 fullscreen.
- Remembers window size/position, theme and view preferences.

## Build

Requires Rust (MSVC toolchain) and the Windows SDK (for `rc.exe`, used to embed the icon).

```
cargo build --release
```

The binary is `target\release\images.exe`. Run it with a file path, or open it and press Ctrl+O.

## Open with / default app

Copy `images.exe` somewhere permanent (for example `%LOCALAPPDATA%\Programs\Images\`), then register it for the current user (no admin needed):

```
images.exe --register     # adds it to "Open with" and Settings > Apps > Default apps
images.exe --unregister   # removes the registration
```

Windows only lets the user choose default apps, so pick Images in **Settings > Apps > Default apps** — or use *Set as default…* in the app's own Settings page, which opens Images' page there directly.

## Tests

```
cargo test --release
```
