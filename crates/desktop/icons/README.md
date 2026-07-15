# App icons

These are **placeholder** icons (solid blue), enough for `cargo check`/`build`
to embed the Windows resource. Replace them with the real logo:

```
cargo tauri icon path/to/logo.png
```

That regenerates `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.ico` and
adds `icon.icns` (needed only for macOS builds — not generated here).
