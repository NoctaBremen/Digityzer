//! Widgets GTK4 compartidos con Spector (feature `gtk`): el espectrograma con sus ejes, la
//! exportación a PNG y el estilo base.

pub mod export;
pub mod spectrogram_view;
pub mod strings;
pub mod style;

pub use spectrogram_view::SpectrogramView;
pub use style::load_css;
