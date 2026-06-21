//! The presentation pipeline (`docs/05-frontends-display-spec.md`): turn an
//! *indexed* Spectrum framebuffer (one logical colour 0–15 per pixel) plus a
//! border colour into RGBA, through a shared theme + effect chain that every head
//! reuses. Standalone — depends on neither `z80` nor `spectrum`. A new theme is a
//! row in a table; a new effect is a stage in the chain; the core never changes.

/// The 16 logical Spectrum colours (0–7 normal, 8–15 bright) as authentic RGB.
/// Bright-off primaries use 0xD7; bright-on use 0xFF; black is black either way.
pub const AUTHENTIC: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00], // black
    [0x00, 0x00, 0xD7], // blue
    [0xD7, 0x00, 0x00], // red
    [0xD7, 0x00, 0xD7], // magenta
    [0x00, 0xD7, 0x00], // green
    [0x00, 0xD7, 0xD7], // cyan
    [0xD7, 0xD7, 0x00], // yellow
    [0xD7, 0xD7, 0xD7], // white
    [0x00, 0x00, 0x00], // bright black
    [0x00, 0x00, 0xFF], // bright blue
    [0xFF, 0x00, 0x00], // bright red
    [0xFF, 0x00, 0xFF], // bright magenta
    [0x00, 0xFF, 0x00], // bright green
    [0x00, 0xFF, 0xFF], // bright cyan
    [0xFF, 0xFF, 0x00], // bright yellow
    [0xFF, 0xFF, 0xFF], // bright white
];

/// The native Spectrum display size, in pixels.
pub const SCREEN_W: usize = 256;
pub const SCREEN_H: usize = 192;

/// How a theme turns a logical colour index into RGB.
#[derive(Clone, Debug)]
pub enum Theme {
    /// Palette remap: substitute an RGB triple for each of the 16 logical colours
    /// (stays colourful — `authentic`, `gameboy`, …).
    Palette(Box<[[u8; 3]; 16]>),
    /// Duotone ramp: collapse each colour to luminance, then lerp between two
    /// endpoints (mono looks — `green-phosphor`, `dark`, `light`, …).
    Duotone { dark: [u8; 3], light: [u8; 3] },
}

impl Theme {
    /// Resolve a logical colour index (0–15) to RGB under this theme.
    #[inline]
    pub fn rgb(&self, index: u8) -> [u8; 3] {
        let i = (index & 0x0f) as usize;
        match self {
            Theme::Palette(p) => p[i],
            Theme::Duotone { dark, light } => {
                let [r, g, b] = AUTHENTIC[i];
                // Rec.601 luma, 0..255.
                let l = (77 * r as u32 + 150 * g as u32 + 29 * b as u32) >> 8;
                lerp(*dark, *light, l as u8)
            }
        }
    }
}

#[inline]
fn lerp(a: [u8; 3], b: [u8; 3], t: u8) -> [u8; 3] {
    let mix = |x: u8, y: u8| (((255 - t as u16) * x as u16 + t as u16 * y as u16) / 255) as u8;
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2])]
}

/// A composable raster post-process. Only `Scanlines` is implemented in software
/// here; richer effects (shadow mask, phosphor persist, bloom, curvature) are
/// GPU-shader stages that live in the capable heads (see the spec §2.2).
#[derive(Clone, Debug)]
pub enum Effect {
    /// Darken every other output row by `0..=100` percent.
    Scanlines { intensity: u8 },
}

/// How much border to include around the 256×192 display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderMode {
    Full,
    Thin,
    Hidden,
}

impl BorderMode {
    /// Border thickness in source pixels (per side).
    fn px(self) -> usize {
        match self {
            BorderMode::Full => 32,
            BorderMode::Thin => 8,
            BorderMode::Hidden => 0,
        }
    }
}

/// Everything a head needs to turn observations into a surface. Passed unchanged
/// to every head; presets are just named `DisplayConfig`s.
#[derive(Clone, Debug)]
pub struct DisplayConfig {
    pub theme: Theme,
    pub effects: Vec<Effect>,
    pub border: BorderMode,
}

impl DisplayConfig {
    /// Real ULA colours, full border, no effects — pixel-perfect.
    pub fn authentic() -> Self {
        Self {
            theme: Theme::Palette(Box::new(AUTHENTIC)),
            effects: vec![],
            border: BorderMode::Full,
        }
    }

    /// Dark mode: light ink on near-black paper (duotone).
    pub fn dark() -> Self {
        Self {
            theme: Theme::Duotone {
                dark: [0x10, 0x12, 0x16],
                light: [0xE6, 0xE6, 0xEA],
            },
            effects: vec![],
            border: BorderMode::Thin,
        }
    }

    /// Light mode: dark ink on off-white paper (duotone).
    pub fn light() -> Self {
        Self {
            theme: Theme::Duotone {
                dark: [0x20, 0x20, 0x20],
                light: [0xF4, 0xF4, 0xF0],
            },
            effects: vec![],
            border: BorderMode::Thin,
        }
    }

    /// Green P1 phosphor terminal, with a soft scanline.
    pub fn terminal() -> Self {
        Self {
            theme: Theme::Duotone {
                dark: [0x00, 0x10, 0x00],
                light: [0x33, 0xFF, 0x66],
            },
            effects: vec![Effect::Scanlines { intensity: 25 }],
            border: BorderMode::Hidden,
        }
    }

    /// Amber P3 phosphor.
    pub fn amber() -> Self {
        Self {
            theme: Theme::Duotone {
                dark: [0x18, 0x08, 0x00],
                light: [0xFF, 0xB0, 0x00],
            },
            effects: vec![Effect::Scanlines { intensity: 25 }],
            border: BorderMode::Hidden,
        }
    }

    /// Game Boy DMG four-shade green (duotone between the lightest/darkest).
    pub fn gameboy() -> Self {
        Self {
            theme: Theme::Duotone {
                dark: [0x0F, 0x38, 0x0F],
                light: [0x9B, 0xBC, 0x0F],
            },
            effects: vec![],
            border: BorderMode::Hidden,
        }
    }

    /// Resolve a preset by name (the value behind MCP `set_display`).
    pub fn preset(name: &str) -> Option<Self> {
        Some(match name {
            "authentic" => Self::authentic(),
            "dark" => Self::dark(),
            "light" => Self::light(),
            "terminal" | "green" | "phosphor" => Self::terminal(),
            "amber" => Self::amber(),
            "gameboy" => Self::gameboy(),
            _ => return None,
        })
    }
}

/// A rendered frame: tightly-packed RGBA plus its dimensions.
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Render an indexed Spectrum framebuffer to RGBA under `cfg`.
///
/// `indexed` is `SCREEN_W * SCREEN_H` logical colour indices (0–15); `border` is
/// the current border colour (0–7). Output size depends on the border mode.
pub fn render(indexed: &[u8], border: u8, cfg: &DisplayConfig) -> Frame {
    assert_eq!(indexed.len(), SCREEN_W * SCREEN_H, "indexed framebuffer size");
    let b = cfg.border.px();
    let width = SCREEN_W + 2 * b;
    let height = SCREEN_H + 2 * b;
    let border_rgb = cfg.theme.rgb(border & 0x07);

    let mut rgba = vec![0u8; width * height * 4];
    for oy in 0..height {
        for ox in 0..width {
            let in_display = ox >= b && ox < b + SCREEN_W && oy >= b && oy < b + SCREEN_H;
            let rgb = if in_display {
                let idx = indexed[(oy - b) * SCREEN_W + (ox - b)];
                cfg.theme.rgb(idx)
            } else {
                border_rgb
            };
            let p = (oy * width + ox) * 4;
            rgba[p] = rgb[0];
            rgba[p + 1] = rgb[1];
            rgba[p + 2] = rgb[2];
            rgba[p + 3] = 0xFF;
        }
    }

    let mut frame = Frame { width, height, rgba };
    for effect in &cfg.effects {
        apply_effect(&mut frame, effect);
    }
    frame
}

fn apply_effect(frame: &mut Frame, effect: &Effect) {
    match effect {
        Effect::Scanlines { intensity } => {
            let keep = 100u16.saturating_sub(*intensity as u16);
            for y in (1..frame.height).step_by(2) {
                for x in 0..frame.width {
                    let p = (y * frame.width + x) * 4;
                    for c in 0..3 {
                        frame.rgba[p + c] = (frame.rgba[p + c] as u16 * keep / 100) as u8;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(index: u8) -> Vec<u8> {
        vec![index; SCREEN_W * SCREEN_H]
    }

    #[test]
    fn authentic_maps_indices_directly() {
        let f = render(&solid(2), 0, &DisplayConfig::authentic()); // 2 = red
        // Centre display pixel should be authentic red.
        let b = BorderMode::Full.px();
        let p = ((b + 10) * f.width + (b + 10)) * 4;
        assert_eq!(&f.rgba[p..p + 3], &[0xD7, 0x00, 0x00]);
    }

    #[test]
    fn duotone_collapses_to_ramp() {
        // Bright white (15, 0xFF) is max luma -> the light endpoint; black (0) ->
        // dark endpoint; normal white (7, 0xD7) sits below the top of the ramp.
        let t = Theme::Duotone {
            dark: [10, 20, 30],
            light: [200, 210, 220],
        };
        assert_eq!(t.rgb(0), [10, 20, 30], "black -> dark");
        assert_eq!(t.rgb(15), [200, 210, 220], "bright white -> light");
        assert!(t.rgb(7)[0] < 200, "normal white is dimmer than bright white");
    }

    #[test]
    fn border_sizing() {
        let full = render(&solid(0), 1, &DisplayConfig::authentic());
        assert_eq!((full.width, full.height), (256 + 64, 192 + 64));
        let hidden = render(
            &solid(0),
            1,
            &DisplayConfig {
                border: BorderMode::Hidden,
                ..DisplayConfig::authentic()
            },
        );
        assert_eq!((hidden.width, hidden.height), (256, 192));
    }

    #[test]
    fn scanlines_darken_odd_rows() {
        let cfg = DisplayConfig {
            theme: Theme::Palette(Box::new(AUTHENTIC)),
            effects: vec![Effect::Scanlines { intensity: 50 }],
            border: BorderMode::Hidden,
        };
        let f = render(&solid(7), 0, &cfg); // white
        let row0 = (0 * f.width) * 4;
        let row1 = (1 * f.width) * 4;
        assert_eq!(f.rgba[row0], 0xD7, "even row unchanged");
        assert!(f.rgba[row1] < 0xD7, "odd row darkened");
    }
}
