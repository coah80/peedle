use crate::value::{JsValue, ObjectId};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Error,
    TypeError,
    ReferenceError,
    SyntaxError,
    RangeError,
    UriError,
    EvalError,
    AggregateError,
    InternalError,
}

impl ErrorKind {
    pub fn name(self) -> &'static str {
        match self {
            ErrorKind::Error => "Error",
            ErrorKind::TypeError => "TypeError",
            ErrorKind::ReferenceError => "ReferenceError",
            ErrorKind::SyntaxError => "SyntaxError",
            ErrorKind::RangeError => "RangeError",
            ErrorKind::UriError => "URIError",
            ErrorKind::EvalError => "EvalError",
            ErrorKind::AggregateError => "AggregateError",
            ErrorKind::InternalError => "InternalError",
        }
    }
}

#[derive(Debug, Clone)]
pub enum JsError {
    Thrown(JsValue),
    Native {
        kind: ErrorKind,
        message: String,
        stack: Vec<String>,
        object: Option<ObjectId>,
    },
}

impl JsError {
    pub fn native(kind: ErrorKind, message: impl Into<String>) -> Self {
        JsError::Native {
            kind,
            message: message.into(),
            stack: Vec::new(),
            object: None,
        }
    }

    pub fn syntax(message: impl Into<String>) -> Self {
        Self::native(ErrorKind::SyntaxError, message)
    }

    pub fn type_err(message: impl Into<String>) -> Self {
        Self::native(ErrorKind::TypeError, message)
    }

    pub fn reference(message: impl Into<String>) -> Self {
        Self::native(ErrorKind::ReferenceError, message)
    }

    pub fn range(message: impl Into<String>) -> Self {
        Self::native(ErrorKind::RangeError, message)
    }

    pub fn uri(message: impl Into<String>) -> Self {
        Self::native(ErrorKind::UriError, message)
    }

    pub fn eval_err(message: impl Into<String>) -> Self {
        Self::native(ErrorKind::EvalError, message)
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::native(ErrorKind::Error, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::native(ErrorKind::InternalError, message)
    }

    pub fn thrown(value: JsValue) -> Self {
        JsError::Thrown(value)
    }

    pub fn kind(&self) -> Option<ErrorKind> {
        match self {
            JsError::Native { kind, .. } => Some(*kind),
            JsError::Thrown(_) => None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            JsError::Native { kind, message, .. } => {
                if message.is_empty() {
                    kind.name().to_string()
                } else {
                    format!("{}: {}", kind.name(), message)
                }
            }
            JsError::Thrown(val) => format!("Uncaught {:?}", val),
        }
    }
}

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for JsError {}
