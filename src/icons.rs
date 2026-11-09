//! Bundled PNG icons: lookup and 16 px menu versions.

use std::fs::File;
use std::path::{Path, PathBuf};

/// Theme icon used when a bundled icon is missing.
pub const FALLBACK_ICON: &str = "network-vpn";

pub const STATUS_ICONS: &[&str] = &[
    "ghost-off",
    "ghost-on",
    "busy-1",
    "busy-2",
    "error",
    "flag-unknown",
];

#[derive(Debug, Clone)]
pub struct Icons {
    /// `None` when no icon directory was found (theme icons are used).
    pub dir: Option<PathBuf>,
}

impl Icons {
    /// First directory containing `ghost-off.png` among:
    /// `override_dir` (`$CYBERGHOST_INDICATOR_ICONS`), `<exe_dir>/../share/cyberghost-indicator/icons`
    /// (~/.local, /usr/local, /usr, unzipped release), `<exe_dir>/share/…`, `<cwd>/assets/icons` (dev).
    pub fn resolve(
        override_dir: Option<PathBuf>,
        exe_dir: Option<&Path>,
        cwd: Option<&Path>,
    ) -> Icons {
        let mut candidates: Vec<PathBuf> = override_dir.into_iter().collect();
        if let Some(d) = exe_dir {
            candidates.push(d.join("../share/cyberghost-indicator/icons"));
            candidates.push(d.join("share/cyberghost-indicator/icons"));
        }
        if let Some(c) = cwd {
            candidates.push(c.join("assets/icons"));
        }
        Icons {
            dir: candidates
                .into_iter()
                .find(|d| d.join("ghost-off.png").is_file())
                .and_then(|d| d.canonicalize().ok()),
        }
    }

    pub fn from_env() -> Icons {
        let exe = std::env::current_exe().ok();
        let cwd = std::env::current_dir().ok();
        Icons::resolve(
            std::env::var_os("CYBERGHOST_INDICATOR_ICONS").map(PathBuf::from),
            exe.as_deref().and_then(Path::parent),
            cwd.as_deref(),
        )
    }

    fn file(&self, name: &str) -> Option<PathBuf> {
        let p = self.dir.as_ref()?.join(format!("{name}.png"));
        p.is_file().then_some(p)
    }

    /// Absolute path usable as a StatusNotifierItem icon name, or the theme fallback.
    /// (A bare name like "flag-FR" would be resolved in the icon theme, where
    /// GTK falls back to a generic "flag" icon.)
    pub fn path(&self, name: &str) -> String {
        self.file(name).map_or_else(
            || FALLBACK_ICON.to_string(),
            |p| p.to_string_lossy().into_owned(),
        )
    }

    /// Flag file for a country code, or the "?" flag.
    pub fn flag_file(&self, cc: &str) -> Option<PathBuf> {
        let valid = cc.len() == 2 && cc.chars().all(|c| c.is_ascii_uppercase());
        valid
            .then(|| self.file(&format!("flag-{cc}")))
            .flatten()
            .or_else(|| self.file("flag-unknown"))
    }

    pub fn flag(&self, cc: &str) -> String {
        self.flag_file(cc).map_or_else(
            || FALLBACK_ICON.to_string(),
            |p| p.to_string_lossy().into_owned(),
        )
    }
}

/// Decode a square RGBA PNG whose side is a multiple of 16, box-downscale it to
/// 16×16 and re-encode. Menu icons are 16 px; sending the 64 px originals made
/// the D-Bus menu layout ~5× bigger and slower to open.
pub fn png_16px(path: &Path) -> Option<Vec<u8>> {
    let dec = png::Decoder::new(std::io::BufReader::new(File::open(path).ok()?));
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let (w, h) = (info.width as usize, info.height as usize);
    if w != h || w < 16 || !w.is_multiple_of(16) {
        return None;
    }
    let f = w / 16;
    let mut out = vec![0u8; 16 * 16 * 4];
    for oy in 0..16 {
        for ox in 0..16 {
            let mut acc = [0u32; 4];
            for y in oy * f..(oy + 1) * f {
                for x in ox * f..(ox + 1) * f {
                    let i = (y * w + x) * 4;
                    let a = buf[i + 3] as u32;
                    for c in 0..3 {
                        acc[c] += buf[i + c] as u32 * a; // alpha-weighted average
                    }
                    acc[3] += a;
                }
            }
            let o = (oy * 16 + ox) * 4;
            for c in 0..3 {
                out[o + c] = acc[c].checked_div(acc[3]).unwrap_or(0) as u8;
            }
            out[o + 3] = (acc[3] / (f * f) as u32) as u8;
        }
    }
    let mut png_out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_out, 16, 16);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::High);
        enc.write_header().ok()?.write_image_data(&out).ok()?;
    }
    Some(png_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/icons")
    }

    fn write_png(path: &Path, w: u32, h: u32, color: png::ColorType, px: &[u8]) {
        let f = std::io::BufWriter::new(File::create(path).unwrap());
        let mut enc = png::Encoder::new(f, w, h);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(px).unwrap();
    }

    fn decode(bytes: &[u8]) -> (png::OutputInfo, Vec<u8>) {
        let mut r = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        (info, buf)
    }

    #[test]
    fn bundled_status_icons_exist() {
        for name in STATUS_ICONS {
            assert!(
                assets().join(format!("{name}.png")).is_file(),
                "missing {name}.png"
            );
        }
    }

    #[test]
    fn bundled_icons_are_64px_rgba_squares() {
        let mut n = 0;
        for e in std::fs::read_dir(assets()).unwrap().flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "png") {
                let (info, _) = decode(&std::fs::read(&p).unwrap());
                assert_eq!((info.width, info.height), (64, 64), "{p:?}");
                assert_eq!(info.color_type, png::ColorType::Rgba, "{p:?}");
                n += 1;
            }
        }
        assert!(n > 200, "only {n} icons bundled");
    }

    #[test]
    fn flags_for_common_countries_are_bundled() {
        for cc in [
            "FR", "US", "GB", "DE", "CH", "JP", "BR", "CA", "AU", "IN", "EU",
        ] {
            assert!(
                assets().join(format!("flag-{cc}.png")).is_file(),
                "missing flag {cc}"
            );
        }
    }

    #[test]
    fn resolve_prefers_override_then_exe_then_cwd() {
        let a = assets();
        let i = Icons::resolve(Some(a.clone()), None, None);
        assert_eq!(i.dir, Some(a.canonicalize().unwrap()));
        // prefix layout: <prefix>/bin + <prefix>/share/cyberghost-indicator/icons
        let d = tempfile::tempdir().unwrap();
        let icons = d.path().join("share/cyberghost-indicator/icons");
        std::fs::create_dir_all(&icons).unwrap();
        std::fs::copy(a.join("ghost-off.png"), icons.join("ghost-off.png")).unwrap();
        std::fs::create_dir_all(d.path().join("bin")).unwrap();
        let i = Icons::resolve(
            Some("/nonexistent".into()),
            Some(&d.path().join("bin")),
            None,
        );
        assert_eq!(i.dir, Some(icons.canonicalize().unwrap()));
        // dev layout: ./assets/icons
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let i = Icons::resolve(None, Some(Path::new("/nonexistent")), Some(&root));
        assert_eq!(i.dir, Some(a.canonicalize().unwrap()));
    }

    #[test]
    fn directory_without_ghost_icon_is_skipped() {
        let d = tempfile::tempdir().unwrap();
        let i = Icons::resolve(Some(d.path().into()), None, None);
        assert_eq!(i.dir, None);
        assert_eq!(i.path("ghost-off"), FALLBACK_ICON);
        assert_eq!(i.flag("FR"), FALLBACK_ICON);
    }

    #[test]
    fn icon_paths_are_absolute() {
        let i = Icons::resolve(Some(assets()), None, None);
        let p = i.path("ghost-on");
        assert!(p.starts_with('/') && p.ends_with("/ghost-on.png"), "{p}");
        assert_eq!(i.path("does-not-exist"), FALLBACK_ICON);
    }

    #[test]
    fn unknown_or_invalid_codes_use_question_flag() {
        let i = Icons::resolve(Some(assets()), None, None);
        assert!(i.flag("FR").ends_with("/flag-FR.png"));
        for cc in ["QQ", "fr", "FRA", "", "../x"] {
            assert!(i.flag(cc).ends_with("/flag-unknown.png"), "{cc}");
        }
    }

    #[test]
    fn downscales_bundled_flag_to_16px() {
        let out = png_16px(&assets().join("flag-FR.png")).unwrap();
        let (info, px) = decode(&out);
        assert_eq!((info.width, info.height), (16, 16));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert!(px.chunks(4).any(|p| p[3] > 0), "image is fully transparent");
        assert!(out.len() < 2048, "16px icon is {} bytes", out.len());
    }

    #[test]
    fn downscale_averages_and_keeps_transparency() {
        let d = tempfile::tempdir().unwrap();
        // 32x32: left half opaque red, right half transparent
        let mut px = Vec::new();
        for _y in 0..32 {
            for x in 0..32 {
                px.extend_from_slice(if x < 16 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 0, 0]
                });
            }
        }
        let p = d.path().join("a.png");
        write_png(&p, 32, 32, png::ColorType::Rgba, &px);
        let (_, out) = decode(&png_16px(&p).unwrap());
        assert_eq!(&out[0..4], &[255, 0, 0, 255]);
        assert_eq!(&out[15 * 4..16 * 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn downscale_rejects_unsupported_images() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("rgb.png");
        write_png(&p, 16, 16, png::ColorType::Rgb, &[0; 16 * 16 * 3]);
        assert!(png_16px(&p).is_none(), "RGB");
        let p = d.path().join("rect.png");
        write_png(&p, 32, 16, png::ColorType::Rgba, &[0; 32 * 16 * 4]);
        assert!(png_16px(&p).is_none(), "not square");
        let p = d.path().join("odd.png");
        write_png(&p, 20, 20, png::ColorType::Rgba, &[0; 20 * 20 * 4]);
        assert!(png_16px(&p).is_none(), "not a multiple of 16");
        let p = d.path().join("tiny.png");
        write_png(&p, 8, 8, png::ColorType::Rgba, &[0; 8 * 8 * 4]);
        assert!(png_16px(&p).is_none(), "smaller than 16");
        let p = d.path().join("junk.png");
        std::fs::write(&p, b"not a png").unwrap();
        assert!(png_16px(&p).is_none(), "garbage");
        assert!(png_16px(&d.path().join("missing.png")).is_none(), "missing");
    }
}
