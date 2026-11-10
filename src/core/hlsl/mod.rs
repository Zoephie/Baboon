//! HLSL as GLSL, for the model preview.
//!
//! Halo 3's kit ships its shaders as source in `.hlsl_include` tags. They are
//! read from the user's kit and translated when needed, so no shader source
//! lives in this repo. The source leans on everything a real compiler offers:
//! pasted macros, overloads, implicit truncation and promotion, structs that
//! pair a sampler with a texture, row-major matrices. So this is a small
//! compiler rather than a text rewrite: [`pre`] preprocesses to tokens,
//! [`parse`] builds a tree, and [`emit`] types it and writes GLSL for what one
//! entry point reaches.

pub(crate) mod emit;
pub(crate) mod parse;
pub(crate) mod pre;
