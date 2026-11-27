pub mod ast;
pub mod bigint;
pub mod builtins;
pub mod bytecode;
pub mod compiler;
pub mod error;
pub mod exec;
pub mod heap;
pub mod lexer;
pub mod parser;
pub mod regexp;
pub mod unicode_id;
pub mod value;
pub mod vm;

pub use error::{ErrorKind, JsError};
pub use heap::{NativeCallback, ObjectKind, SharedBufferBacking};
pub use value::{EnvId, JsValue, ObjectId, PropertyDescriptor, PropertyKey, TypedArrayKind};
pub use vm::{JsRuntime, ModuleLoader};

pub fn version() -> &'static str {
    "0.1.0"
}
