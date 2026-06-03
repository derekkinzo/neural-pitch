# Icons

`icon.png` is the placeholder source. Regenerate the desktop icon set with:

```sh
npx tauri icon src-tauri/icons/icon.png
```

Then keep only the variants enumerated in `src-tauri/tauri.conf.json` `bundle.icon` (currently `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns`, `icon.ico`). Mobile and Microsoft Store derivatives are regenerated when Phase 6 (mobile) lands.
