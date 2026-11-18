//! Temas de color (T2.9): once roles por tema, diez temas oscuros con sus paletas oficiales.
//!
//! Código propio de Digityzer (no copiado de Spector), en `audio-core` para que Spector pueda
//! adoptarlo más adelante en su propio proyecto. Ningún widget lleva colores fijos: todos leen
//! los roles de [`Theme`].
//!
//! - [`THEMES`]: los valores tal cual las tablas del plan («Temas de color»).
//! - [`Theme::effective`]: el tema que se usa, con el ajuste de contraste mínimo (texto ≥ 4,5:1
//!   sobre fondo y gráfica; tenue ≥ 3:1 sobre gráfica). Solo cambia los roles que no llegan,
//!   mezclándolos hacia `texto` en pasos del 5 %. Ver [`Theme::adjustments`]. Hoy cambia tres
//!   roles: `tenue` de Nord (#616e88 → #768198, 3,19:1), de Tokyo Night (#565f89 → #5b648e,
//!   3,13:1) y de Ayu Dark (#565b66 → #5b606a, 3,01:1).
//! - [`Theme::gtk_css`]: CSS de GTK con los roles como `@define-color` y reglas para la ventana.
//! - [`Theme::waterfall_gradient`]: degradado de cinco paradas del espectrograma en cascada.

use std::fmt;

/// Color sRGB de 8 bits por canal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Desde `0xRRGGBB`.
    pub const fn hex(v: u32) -> Self {
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// Componentes en [0, 1] para cairo.
    pub fn unit(self) -> (f64, f64, f64) {
        (
            f64::from(self.0) / 255.0,
            f64::from(self.1) / 255.0,
            f64::from(self.2) / 255.0,
        )
    }

    /// Luminancia relativa (WCAG 2.x).
    pub fn luminance(self) -> f64 {
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.0) + 0.7152 * lin(self.1) + 0.0722 * lin(self.2)
    }

    /// Mezcla lineal en sRGB: `t = 0` → `self`, `t = 1` → `other`.
    pub fn mix(self, other: Rgb, t: f64) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// Relación de contraste WCAG entre dos colores (1–21).
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Contraste mínimo de `texto` sobre `fondo` y `gráfica`.
pub const MIN_TEXT_CONTRAST: f64 = 4.5;
/// Contraste mínimo de `tenue` sobre `gráfica`.
pub const MIN_DIM_CONTRAST: f64 = 3.0;

/// Un tema: los once roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Identificador estable (se guarda en la configuración).
    pub id: &'static str,
    /// Nombre visible.
    pub name: &'static str,
    /// Ventana, barras y controles.
    pub fondo: Rgb,
    /// Fondo de medidores, espectro, goniómetro y forma de onda.
    pub grafica: Rgb,
    /// Rejilla, bordes y separadores.
    pub rejilla: Rgb,
    /// Texto principal, valores y cronómetro.
    pub texto: Rgb,
    /// Etiquetas de ejes, unidades y texto secundario.
    pub tenue: Rgb,
    /// Espectro instantáneo, forma de onda y botón activo.
    pub curva: Rgb,
    /// Capa media del espectro.
    pub media: Rgb,
    /// Retención de picos y cursor.
    pub picos: Rgb,
    /// Medidor hasta −18 dBFS, goniómetro, correlación sana, «grabando sin xruns».
    pub ok: Rgb,
    /// Medidor de −18 a −3 dBFS, correlación baja, avisos del perfil de aguja.
    pub aviso: Rgb,
    /// Clipping, xruns, correlación negativa y botón Grabar.
    pub clip: Rgb,
}

#[allow(clippy::too_many_arguments)]
const fn t(id: &'static str, name: &'static str, ui: [u32; 5], graph: [u32; 6]) -> Theme {
    Theme {
        id,
        name,
        fondo: Rgb::hex(ui[0]),
        grafica: Rgb::hex(ui[1]),
        rejilla: Rgb::hex(ui[2]),
        texto: Rgb::hex(ui[3]),
        tenue: Rgb::hex(ui[4]),
        curva: Rgb::hex(graph[0]),
        media: Rgb::hex(graph[1]),
        picos: Rgb::hex(graph[2]),
        ok: Rgb::hex(graph[3]),
        aviso: Rgb::hex(graph[4]),
        clip: Rgb::hex(graph[5]),
    }
}

/// Los diez temas, con los valores de las tablas del plan. Dracula es el primero y el de por
/// defecto.
pub const THEMES: [Theme; 10] = [
    t(
        "dracula",
        "Dracula",
        [0x282a36, 0x21222c, 0x44475a, 0xf8f8f2, 0x6272a4],
        [0xbd93f9, 0x8be9fd, 0xff79c6, 0x50fa7b, 0xffb86c, 0xff5555],
    ),
    t(
        "catppuccin-mocha",
        "Catppuccin Mocha",
        [0x1e1e2e, 0x181825, 0x313244, 0xcdd6f4, 0x6c7086],
        [0x89b4fa, 0x94e2d5, 0xf5c2e7, 0xa6e3a1, 0xf9e2af, 0xf38ba8],
    ),
    t(
        "gruvbox-dark",
        "Gruvbox Dark",
        [0x282828, 0x1d2021, 0x3c3836, 0xebdbb2, 0x928374],
        [0x83a598, 0x8ec07c, 0xd3869b, 0xb8bb26, 0xfabd2f, 0xfb4934],
    ),
    t(
        "nord",
        "Nord",
        [0x3b4252, 0x2e3440, 0x434c5e, 0xeceff4, 0x616e88],
        [0x88c0d0, 0x8fbcbb, 0xb48ead, 0xa3be8c, 0xebcb8b, 0xbf616a],
    ),
    t(
        "tokyo-night",
        "Tokyo Night",
        [0x1a1b26, 0x16161e, 0x292e42, 0xc0caf5, 0x565f89],
        [0x7aa2f7, 0x7dcfff, 0xbb9af7, 0x9ece6a, 0xe0af68, 0xf7768e],
    ),
    t(
        "rose-pine",
        "Rosé Pine",
        [0x191724, 0x1f1d2e, 0x26233a, 0xe0def4, 0x6e6a86],
        [0xc4a7e7, 0x9ccfd8, 0xebbcba, 0x31748f, 0xf6c177, 0xeb6f92],
    ),
    t(
        "everforest-dark",
        "Everforest Dark",
        [0x2d353b, 0x272e33, 0x3d484d, 0xd3c6aa, 0x859289],
        [0x7fbbb3, 0x83c092, 0xd699b6, 0xa7c080, 0xdbbc7f, 0xe67e80],
    ),
    t(
        "kanagawa-wave",
        "Kanagawa Wave",
        [0x1f1f28, 0x16161d, 0x363646, 0xdcd7ba, 0x727169],
        [0x7e9cd8, 0x7aa89f, 0x957fb8, 0x98bb6c, 0xe6c384, 0xe46876],
    ),
    t(
        "monokai-pro",
        "Monokai Pro",
        [0x2d2a2e, 0x221f22, 0x403e41, 0xfcfcfa, 0x727072],
        [0x78dce8, 0xab9df2, 0xfc9867, 0xa9dc76, 0xffd866, 0xff6188],
    ),
    t(
        "ayu-dark",
        "Ayu Dark",
        [0x0b0e14, 0x0d1017, 0x131721, 0xbfbdb6, 0x565b66],
        [0x59c2ff, 0x95e6cb, 0xd2a6ff, 0xaad94c, 0xe6b450, 0xd95757],
    ),
];

/// Un rol cambiado por el ajuste de contraste.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adjustment {
    /// Rol (`tenue` o `texto`).
    pub role: &'static str,
    /// Valor de la tabla.
    pub from: Rgb,
    /// Valor usado.
    pub to: Rgb,
    /// Contraste conseguido sobre la superficie más exigente.
    pub contrast: f64,
}

/// Mezcla `c` hacia `target` en pasos del 5 % hasta que su contraste con todas las `surfaces`
/// llegue a `min`.
fn raise(c: Rgb, target: Rgb, surfaces: &[Rgb], min: f64) -> Rgb {
    let ok = |x: Rgb| surfaces.iter().all(|&s| contrast(x, s) >= min);
    (0..=20)
        .map(|k| c.mix(target, f64::from(k) * 0.05))
        .find(|&x| ok(x))
        .unwrap_or(target)
}

impl Theme {
    /// El tema por defecto (Dracula).
    pub fn default_theme() -> &'static Theme {
        &THEMES[0]
    }

    /// Tema por identificador; si no existe, el de por defecto.
    pub fn by_id(id: &str) -> &'static Theme {
        THEMES
            .iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| Self::default_theme())
    }

    /// Los once roles con su nombre, en el orden de la tabla.
    pub fn roles(&self) -> [(&'static str, Rgb); 11] {
        [
            ("fondo", self.fondo),
            ("grafica", self.grafica),
            ("rejilla", self.rejilla),
            ("texto", self.texto),
            ("tenue", self.tenue),
            ("curva", self.curva),
            ("media", self.media),
            ("picos", self.picos),
            ("ok", self.ok),
            ("aviso", self.aviso),
            ("clip", self.clip),
        ]
    }

    /// El tema con el contraste mínimo garantizado (T2.9b).
    pub fn effective(&self) -> Theme {
        let mut t = *self;
        t.texto = raise(
            t.texto,
            Rgb(255, 255, 255),
            &[t.fondo, t.grafica],
            MIN_TEXT_CONTRAST,
        );
        t.tenue = raise(t.tenue, t.texto, &[t.grafica], MIN_DIM_CONTRAST);
        t
    }

    /// Roles que [`Self::effective`] cambia, para documentarlos.
    pub fn adjustments(&self) -> Vec<Adjustment> {
        let e = self.effective();
        let mut v = Vec::new();
        if e.texto != self.texto {
            v.push(Adjustment {
                role: "texto",
                from: self.texto,
                to: e.texto,
                contrast: contrast(e.texto, e.fondo).min(contrast(e.texto, e.grafica)),
            });
        }
        if e.tenue != self.tenue {
            v.push(Adjustment {
                role: "tenue",
                from: self.tenue,
                to: e.tenue,
                contrast: contrast(e.tenue, e.grafica),
            });
        }
        v
    }

    /// Degradado del espectrograma en cascada: gráfica → curva → picos → aviso → texto. Las
    /// cuatro últimas paradas se ordenan por luminancia para que la intensidad crezca siempre
    /// (en Catppuccin Mocha, por ejemplo, el aviso es más claro que el texto y va al final).
    pub fn waterfall_gradient(&self) -> [Rgb; 5] {
        let mut s = [self.curva, self.picos, self.aviso, self.texto];
        s.sort_by(|a, b| a.luminance().total_cmp(&b.luminance()));
        [self.grafica, s[0], s[1], s[2], s[3]]
    }

    /// Color del degradado en `x` ∈ [0, 1].
    pub fn waterfall_at(&self, x: f64) -> Rgb {
        let g = self.waterfall_gradient();
        let x = x.clamp(0.0, 1.0) * 4.0;
        let i = (x.floor() as usize).min(3);
        g[i].mix(g[i + 1], x - i as f64)
    }

    /// CSS de GTK para toda la ventana (T2.9c): los roles como `@define-color`, los colores con
    /// nombre del tema de GTK redefinidos con ellos, y reglas para ventana, botones, campos y
    /// menús. Se carga con prioridad de aplicación.
    pub fn gtk_css(&self) -> String {
        let mut s = String::new();
        for (name, c) in self.roles() {
            s.push_str(&format!("@define-color dz_{name} {c};\n"));
        }
        let surface = self.fondo.mix(self.rejilla, 0.5);
        let hover = self.rejilla.mix(self.texto, 0.12);
        s.push_str(&format!(
            "@define-color theme_bg_color {f};
@define-color theme_fg_color {x};
@define-color theme_base_color {g};
@define-color theme_text_color {x};
@define-color theme_selected_bg_color {c};
@define-color theme_selected_fg_color {g};
@define-color insensitive_fg_color {d};
@define-color borders {r};
@define-color window_bg_color {f};
@define-color window_fg_color {x};
@define-color view_bg_color {g};
@define-color view_fg_color {x};
@define-color accent_color {c};
@define-color accent_bg_color {c};
@define-color accent_fg_color {g};
@define-color error_color {k};
@define-color warning_color {a};
@define-color success_color {o};
window, .background {{ background-color: {f}; color: {x}; }}
label {{ color: {x}; }}
.dim-label, label.dim-label {{ color: {d}; opacity: 1; }}
button, dropdown > button, menubutton > button {{
  background-image: none; background-color: {sf}; color: {x}; border-color: {r};
  box-shadow: none; text-shadow: none; }}
button:hover {{ background-color: {hv}; }}
button:checked, button:active {{ background-color: {c}; color: {g}; }}
button:disabled, button:disabled label {{ color: {d}; background-color: {f}; }}
button.destructive-action {{ background-color: {k}; color: {g}; }}
button.destructive-action:disabled {{ background-color: {f}; color: {d}; }}
entry {{ background-image: none; background-color: {g}; color: {x}; border-color: {r};
  caret-color: {c}; box-shadow: none; }}
entry:focus-within {{ border-color: {c}; }}
entry > text > placeholder {{ color: {d}; }}
popover > contents, popover > arrow {{ background-color: {f}; color: {x}; border-color: {r}; }}
listview, list {{ background-color: {f}; color: {x}; }}
row:selected, listview > row:selected {{ background-color: {c}; color: {g}; }}
.rec-time {{ color: {x}; }}
.rec-time.recording {{ color: {k}; }}
.status-ok {{ color: {o}; }}
.status-warn {{ color: {a}; }}
.status-clip {{ color: {k}; }}
",
            f = self.fondo,
            g = self.grafica,
            r = self.rejilla,
            x = self.texto,
            d = self.tenue,
            c = self.curva,
            o = self.ok,
            a = self.aviso,
            k = self.clip,
            sf = surface,
            hv = hover,
        ));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// Paletas oficiales de cada tema, escritas a partir de las fuentes oficiales y no copiadas
    /// de [`THEMES`]: si alguien cambia un valor de la tabla por error, este test lo detecta.
    ///
    /// Fuentes: Dracula (github.com/dracula/dracula-theme; #21222c y #191a21 son los fondos
    /// oscuros de spec.draculatheme.com, que no se pudo consultar desde el entorno de desarrollo),
    /// Catppuccin (catppuccin.com/palette, Mocha), Gruvbox (morhetz/gruvbox), Nord
    /// (nordtheme.com, nord0–nord15), Tokyo Night (folke/tokyonight.nvim,
    /// extras/lua/tokyonight_night.lua), Rosé Pine (rosepinetheme.com/palette), Everforest
    /// (sainnhe/everforest palette.md, variantes medium y hard), Kanagawa (rebelot/kanagawa.nvim),
    /// Monokai Pro (monokai.pro, filtro clásico) y Ayu (ayu-theme/ayu-colors, dark).
    fn official() -> HashMap<&'static str, HashSet<u32>> {
        let p: &[(&str, &[u32])] = &[
            (
                "dracula",
                &[
                    0x282a36, 0x44475a, 0xf8f8f2, 0x6272a4, 0x8be9fd, 0x50fa7b, 0xffb86c, 0xff79c6,
                    0xbd93f9, 0xff5555, 0xf1fa8c, 0x21222c, 0x191a21,
                ],
            ),
            (
                "catppuccin-mocha",
                &[
                    0xf5e0dc, 0xf2cdcd, 0xf5c2e7, 0xcba6f7, 0xf38ba8, 0xeba0ac, 0xfab387, 0xf9e2af,
                    0xa6e3a1, 0x94e2d5, 0x89dceb, 0x74c7ec, 0x89b4fa, 0xb4befe, 0xcdd6f4, 0xbac2de,
                    0xa6adc8, 0x9399b2, 0x7f849c, 0x6c7086, 0x585b70, 0x45475a, 0x313244, 0x1e1e2e,
                    0x181825, 0x11111b,
                ],
            ),
            (
                "gruvbox-dark",
                &[
                    0x1d2021, 0x282828, 0x32302f, 0x3c3836, 0x504945, 0x665c54, 0x7c6f64, 0x928374,
                    0xa89984, 0xbdae93, 0xd5c4a1, 0xebdbb2, 0xfbf1c7, 0xcc241d, 0x98971a, 0xd79921,
                    0x458588, 0xb16286, 0x689d6a, 0xd65d0e, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598,
                    0xd3869b, 0x8ec07c, 0xfe8019,
                ],
            ),
            (
                "nord",
                &[
                    0x2e3440, 0x3b4252, 0x434c5e, 0x4c566a, 0xd8dee9, 0xe5e9f0, 0xeceff4, 0x8fbcbb,
                    0x88c0d0, 0x81a1c1, 0x5e81ac, 0xbf616a, 0xd08770, 0xebcb8b, 0xa3be8c, 0xb48ead,
                ],
            ),
            (
                "tokyo-night",
                &[
                    0x1a1b26, 0x16161e, 0x292e42, 0xc0caf5, 0xa9b1d6, 0x565f89, 0x545c7e, 0x737aa2,
                    0x414868, 0x7aa2f7, 0x7dcfff, 0xbb9af7, 0x9ece6a, 0xe0af68, 0xf7768e, 0xff9e64,
                    0x2ac3de, 0x73daca, 0xb4f9f8, 0x1abc9c,
                ],
            ),
            (
                "rose-pine",
                &[
                    0x191724, 0x1f1d2e, 0x26233a, 0x6e6a86, 0x908caa, 0xe0def4, 0xeb6f92, 0xf6c177,
                    0xebbcba, 0x31748f, 0x9ccfd8, 0xc4a7e7, 0x21202e, 0x403d52, 0x524f67,
                ],
            ),
            (
                "everforest-dark",
                &[
                    // Medium.
                    0x232a2e, 0x2d353b, 0x343f44, 0x3d484d, 0x475258, 0x4f585e, 0x56635f,
                    // Hard (bg_dim y bg0).
                    0x1e2326, 0x272e33, 0xd3c6aa, 0xe67e80, 0xe69875, 0xdbbc7f, 0xa7c080, 0x83c092,
                    0x7fbbb3, 0xd699b6, 0x7a8478, 0x859289, 0x9da9a0,
                ],
            ),
            (
                "kanagawa-wave",
                &[
                    0x16161d, 0x181820, 0x1a1a22, 0x1f1f28, 0x2a2a37, 0x363646, 0x54546d, 0xdcd7ba,
                    0xc8c093, 0x727169, 0x7e9cd8, 0x7fb4ca, 0x7aa89f, 0x6a9589, 0x957fb8, 0x938aa9,
                    0x98bb6c, 0xe6c384, 0xc0a36e, 0xffa066, 0xe46876, 0xc34043, 0xe82424, 0xd27e99,
                ],
            ),
            (
                "monokai-pro",
                &[
                    0x19181a, 0x221f22, 0x2d2a2e, 0xfcfcfa, 0xff6188, 0xfc9867, 0xffd866, 0xa9dc76,
                    0x78dce8, 0xab9df2, 0xc1c0c0, 0x939293, 0x727072, 0x5b595c, 0x403e41,
                ],
            ),
            (
                "ayu-dark",
                &[
                    0x0b0e14, 0x0d1017, 0x131721, 0x11151c, 0x0f131a, 0xbfbdb6, 0x565b66, 0x39bae6,
                    0xffb454, 0x59c2ff, 0xaad94c, 0x95e6cb, 0xf07178, 0xff8f40, 0xe6b673, 0xd2a6ff,
                    0xf29668, 0xe6b450, 0xd95757,
                ],
            ),
        ];
        p.iter()
            .map(|(id, cs)| (*id, cs.iter().copied().collect()))
            .collect()
    }

    /// Valores de la tabla que no están en la paleta oficial de su tema.
    fn outside_official() -> Vec<(&'static str, &'static str, Rgb)> {
        let off = official();
        let mut v = Vec::new();
        for th in &THEMES {
            let set = &off[th.id];
            for (role, c) in th.roles() {
                let key = (u32::from(c.0) << 16) | (u32::from(c.1) << 8) | u32::from(c.2);
                if !set.contains(&key) {
                    v.push((th.id, role, c));
                }
            }
        }
        v
    }

    /// T2.9a: los diez temas cargan y cada valor está en su paleta oficial. Única excepción
    /// conocida: el `tenue` de Nord (#616e88) no está en nord0–nord15; viene de ports como
    /// nord-vim («nord3 bright»). Se deja como en la tabla del plan y el ajuste de contraste lo
    /// sustituye igualmente.
    #[test]
    fn temas_y_paletas_oficiales() {
        assert_eq!(THEMES.len(), 10);
        let ids: HashSet<_> = THEMES.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), 10);
        assert_eq!(Theme::default_theme().name, "Dracula");
        assert_eq!(Theme::by_id("nord").name, "Nord");
        assert_eq!(Theme::by_id("no-existe").id, "dracula");
        assert_eq!(
            outside_official(),
            vec![("nord", "tenue", Rgb::hex(0x616e88))]
        );
    }

    /// T2.9b: contraste mínimo en los diez temas, ajustando solo los roles que no llegan.
    #[test]
    fn contraste_minimo() {
        for th in &THEMES {
            let e = th.effective();
            assert!(
                contrast(e.texto, e.fondo) >= MIN_TEXT_CONTRAST,
                "{}",
                th.name
            );
            assert!(
                contrast(e.texto, e.grafica) >= MIN_TEXT_CONTRAST,
                "{}",
                th.name
            );
            assert!(
                contrast(e.tenue, e.grafica) >= MIN_DIM_CONTRAST,
                "{}",
                th.name
            );
            // Solo cambian texto y tenue.
            for ((n, a), (_, b)) in th.roles().iter().zip(e.roles()) {
                if *n != "texto" && *n != "tenue" {
                    assert_eq!(*a, b, "{} {n}", th.name);
                }
            }
        }
        let adjusted: Vec<(&str, &str)> = THEMES
            .iter()
            .flat_map(|t| t.adjustments().into_iter().map(move |a| (t.id, a.role)))
            .collect();
        assert_eq!(
            adjusted,
            vec![
                ("nord", "tenue"),
                ("tokyo-night", "tenue"),
                ("ayu-dark", "tenue")
            ]
        );
    }

    #[test]
    fn contraste_wcag_conocido() {
        assert!((contrast(Rgb(0, 0, 0), Rgb(255, 255, 255)) - 21.0).abs() < 1e-9);
        assert!((contrast(Rgb(1, 2, 3), Rgb(1, 2, 3)) - 1.0).abs() < 1e-12);
        assert_eq!(Rgb::hex(0x6272a4).to_string(), "#6272a4");
        assert_eq!(Rgb(0, 0, 0).mix(Rgb(200, 100, 50), 0.5), Rgb(100, 50, 25));
    }

    #[test]
    fn degradado_con_luminancia_creciente() {
        for th in &THEMES {
            let g = th.waterfall_gradient();
            for w in g.windows(2) {
                assert!(w[0].luminance() <= w[1].luminance(), "{}: {g:?}", th.name);
            }
            assert_eq!(th.waterfall_at(0.0), th.grafica);
            assert_eq!(th.waterfall_at(1.0), g[4]);
            assert!(g.contains(&th.texto) && g.contains(&th.aviso));
        }
    }

    #[test]
    fn css_con_todos_los_roles() {
        let css = THEMES[0].effective().gtk_css();
        for (name, c) in THEMES[0].effective().roles() {
            assert!(
                css.contains(&format!("@define-color dz_{name} {c};")),
                "{name}"
            );
        }
        assert!(css.contains("window, .background { background-color: #282a36;"));
    }
}
