//! Runtime values. Value semantics everywhere: lists and structs are immutable `Rc`s with
//! copy-on-write (`Rc::make_mut`), so sharing is free and mutation never aliases.

use crate::rt::ScopeId;
use std::rc::Rc;
use vard_lang::ast::ExprId;

#[derive(Clone, Debug)]
pub struct StructVal {
    pub name: Rc<str>,
    pub fields: Vec<(Rc<str>, Value)>,
}

#[derive(Debug)]
pub enum Func {
    Lambda { expr: ExprId, scope: ScopeId },
    Global(usize),
    ViewFn { view: usize, member: usize, scope: ScopeId },
    Extern(usize),
}

#[derive(Clone, Debug)]
pub enum Value {
    Unit,
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    /// RGBA
    Color(u32),
    Enum(Rc<str>),
    List(Rc<Vec<Value>>),
    /// String-keyed map in insertion order.
    Map(Rc<Vec<(Rc<str>, Value)>>),
    Struct(Rc<StructVal>),
    Func(Rc<Func>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        use Value::*;
        match (self, other) {
            (Unit, Unit) | (Nil, Nil) => true,
            (Bool(a), Bool(b)) => a == b,
            (Int(a), Int(b)) => a == b,
            (Float(a), Float(b)) => a == b,
            (Int(a), Float(b)) | (Float(b), Int(a)) => (*a as f64) == *b,
            (Str(a), Str(b)) => a == b,
            (Color(a), Color(b)) => a == b,
            (Enum(a), Enum(b)) => a == b,
            (List(a), List(b)) => Rc::ptr_eq(a, b) || a == b,
            (Map(a), Map(b)) => {
                Rc::ptr_eq(a, b) || (a.len() == b.len() && a.iter().all(|(k, v)| b.iter().any(|(k2, v2)| k == k2 && v == v2)))
            }
            (Struct(a), Struct(b)) => {
                Rc::ptr_eq(a, b) || (a.name == b.name && a.fields.len() == b.fields.len() && a.fields.iter().zip(&b.fields).all(|(x, y)| x.1 == y.1))
            }
            (Func(a), Func(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(Rc::from(s))
    }
    pub fn map_get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(m) => m.iter().find(|(k, _)| &**k == key).map(|(_, v)| v),
            Value::Struct(s) => s.fields.iter().find(|(k, _)| &**k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn truthy(&self) -> bool {
        matches!(self, Value::Bool(true))
    }
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Value::Int(i) => Some(*i as f32),
            Value::Float(f) => Some(*f as f32),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }
    /// Text used by string interpolation.
    pub fn display(&self) -> String {
        match self {
            Value::Str(s) => s.to_string(),
            other => other.repr(),
        }
    }
    /// Compact literal-like form used by the inspector (`state` dumps, logs).
    pub fn repr(&self) -> String {
        match self {
            Value::Unit => "()".into(),
            Value::Nil => "nil".into(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{:.1}", f)
                } else {
                    f.to_string()
                }
            }
            Value::Str(s) => format!("{:?}", &**s),
            Value::Color(c) => format!("#{:08X}", c),
            Value::Enum(e) => format!(".{}", e),
            Value::List(l) => format!("[{}]", l.iter().map(|v| v.repr()).collect::<Vec<_>>().join(", ")),
            Value::Map(m) if m.is_empty() => "[:]".into(),
            Value::Map(m) => format!("[{}]", m.iter().map(|(k, v)| format!("{:?}: {}", &**k, v.repr())).collect::<Vec<_>>().join(", ")),
            Value::Struct(s) => {
                format!("{}({})", s.name, s.fields.iter().map(|(k, v)| format!("{}: {}", k, v.repr())).collect::<Vec<_>>().join(", "))
            }
            Value::Func(_) => "fn".into(),
        }
    }
}
