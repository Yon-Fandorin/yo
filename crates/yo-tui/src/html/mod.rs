//! Deterministic browser projection of completed surface state.

mod escape;
mod projection;
mod style;

pub use projection::HtmlSurface;

#[cfg(test)]
mod tests;
