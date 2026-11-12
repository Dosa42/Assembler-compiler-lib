pub mod c_parser;
pub mod fidelity;
pub mod linux_converter;
pub mod linux_preprocessor;
pub mod stmt_converter;
pub mod type_map;
pub mod uagl2c;
pub mod uagl_emitter;
pub mod validator;

#[cfg(target_arch = "wasm32")]
mod wasm;
