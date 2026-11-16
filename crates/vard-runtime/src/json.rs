//! Values <-> JSON at the native boundary (extern calls, callbacks, events, `pref` storage).
//!
//! Encoding is shape-preserving and type-free: structs and maps become objects, enums their
//! variant name, colors `"#RRGGBBAA"`, `nil` `null`, and function values become
//! `{"$callback": id}` handles the host can invoke later. Decoding is guided by the declared
//! Vard type, so a host may answer loosely (a number as a string, extra fields, missing
//! fields with defaults) and the app still receives a well-typed value.

use crate::rt::Runtime;
use crate::value::{StructVal, Value};
use serde_json::{Map, Number, Value as J};
use std::rc::Rc;
use vard_lang::ast::{Program, TypeRef};

pub type Json = J;

/// Encode a value. `callback` registers a function value and returns its handle id.
pub fn to_json(v: &Value, callback: &mut dyn FnMut(&Value) -> u64) -> J {
    match v {
        Value::Unit | Value::Nil => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => J::Number((*i).into()),
        Value::Float(f) => Number::from_f64(*f).map(J::Number).unwrap_or(J::Null),
        Value::Str(s) => J::String(s.to_string()),
        Value::Color(c) => J::String(format!("#{:08X}", c)),
        Value::Enum(e) => J::String(e.to_string()),
        Value::List(l) => J::Array(l.iter().map(|x| to_json(x, callback)).collect()),
        Value::Map(m) => J::Object(m.iter().map(|(k, x)| (k.to_string(), to_json(x, callback))).collect()),
        Value::Struct(s) => J::Object(s.fields.iter().map(|(k, x)| (k.to_string(), to_json(x, callback))).collect()),
        Value::Func(_) => {
            let id = callback(v);
            let mut m = Map::new();
            m.insert("$callback".into(), J::Number(id.into()));
            J::Object(m)
        }
    }
}

/// Encode data (no callbacks; a function value becomes null).
pub fn data_to_json(v: &Value) -> J {
    to_json(v, &mut |_| 0)
}

/// Untyped decoding (mocks without a known type, `emit` payloads before typing).
pub fn from_json_any(j: &J) -> Value {
    match j {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or(0.0)),
        },
        J::String(s) => Value::str(s),
        J::Array(a) => Value::List(Rc::new(a.iter().map(from_json_any).collect())),
        J::Object(o) => Value::Map(Rc::new(o.iter().map(|(k, v)| (Rc::from(k.as_str()), from_json_any(v))).collect())),
    }
}

/// Parse a token from the action language: JSON if it parses, otherwise a plain string.
pub fn parse_loose(s: &str) -> J {
    serde_json::from_str(s).unwrap_or_else(|_| J::String(s.to_string()))
}

pub fn parse_color(s: &str) -> Option<u32> {
    let h = s.trim().strip_prefix('#')?;
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(match h.len() {
        3 => {
            let (r, g, b) = ((v >> 8) & 0xF, (v >> 4) & 0xF, v & 0xF);
            (r * 17) << 24 | (g * 17) << 16 | (b * 17) << 8 | 0xFF
        }
        6 => v << 8 | 0xFF,
        8 => v,
        _ => return None,
    })
}

impl Runtime {
    /// Decode `j` as a value of Vard type `t` (missing struct fields take their defaults).
    pub(crate) fn decode_json(&mut self, p: &Program, j: &J, t: &TypeRef) -> Value {
        match t {
            TypeRef::Opt(inner) => {
                if j.is_null() {
                    Value::Nil
                } else {
                    self.decode_json(p, j, inner)
                }
            }
            TypeRef::List(inner) => match j {
                J::Array(a) => Value::List(Rc::new(a.iter().map(|x| self.decode_json(p, x, inner)).collect())),
                _ => Value::List(Rc::new(Vec::new())),
            },
            TypeRef::Map(inner) => match j {
                J::Object(o) => Value::Map(Rc::new(o.iter().map(|(k, x)| (Rc::from(k.as_str()), self.decode_json(p, x, inner))).collect())),
                _ => Value::Map(Rc::new(Vec::new())),
            },
            TypeRef::Fn(..) => Value::Nil,
            TypeRef::Named(n, _) => match n.as_str() {
                "Int" => match j {
                    J::Number(x) => Value::Int(x.as_i64().unwrap_or_else(|| x.as_f64().unwrap_or(0.0) as i64)),
                    J::String(s) => s.trim().parse().map(Value::Int).unwrap_or(Value::Int(0)),
                    J::Bool(b) => Value::Int(*b as i64),
                    _ => Value::Int(0),
                },
                "Float" => match j {
                    J::Number(x) => Value::Float(x.as_f64().unwrap_or(0.0)),
                    J::String(s) => s.trim().parse().map(Value::Float).unwrap_or(Value::Float(0.0)),
                    _ => Value::Float(0.0),
                },
                "String" => match j {
                    J::String(s) => Value::str(s),
                    J::Null => Value::str(""),
                    other => Value::str(&other.to_string()),
                },
                "Bool" => match j {
                    J::Bool(b) => Value::Bool(*b),
                    J::String(s) => Value::Bool(s == "true" || s == "1"),
                    J::Number(x) => Value::Bool(x.as_f64().unwrap_or(0.0) != 0.0),
                    _ => Value::Bool(false),
                },
                "Color" => match j {
                    J::String(s) => Value::Color(parse_color(s).unwrap_or(0x000000FF)),
                    J::Number(x) => Value::Color(x.as_u64().unwrap_or(0) as u32),
                    _ => Value::Color(0x000000FF),
                },
                name => {
                    if let Some(&si) = p.struct_index.get(name) {
                        let decl = &p.structs[si];
                        let root = self.root_scope;
                        let fields = decl
                            .fields
                            .iter()
                            .map(|f| {
                                let v = match (j.get(&f.name), f.default) {
                                    (Some(x), _) if !x.is_null() || matches!(f.ty, TypeRef::Opt(_)) => self.decode_json(p, x, &f.ty),
                                    (_, Some(d)) => self.untracked(|rt| rt.eval(p, d, root)),
                                    (x, None) => self.decode_json(p, x.unwrap_or(&J::Null), &f.ty),
                                };
                                (Rc::from(f.name.as_str()), v)
                            })
                            .collect();
                        return Value::Struct(Rc::new(StructVal { name: Rc::from(name), fields }));
                    }
                    let variants: Option<Vec<String>> = p
                        .enums
                        .iter()
                        .find(|e| e.name == name)
                        .map(|e| e.variants.clone())
                        .or_else(|| vard_lang::registry::enum_variants(name).map(|v| v.iter().map(|s| s.to_string()).collect()));
                    if let Some(vs) = variants {
                        let s = j.as_str().unwrap_or("");
                        return match vs.iter().find(|v| v.eq_ignore_ascii_case(s)) {
                            Some(v) => Value::Enum(Rc::from(v.as_str())),
                            None => {
                                self.error("R0011", vard_lang::diag::Span::default(), format!("native value {} is not a variant of {} ({})", j, name, vs.join(" ")));
                                Value::Enum(Rc::from(vs.first().map(|s| s.as_str()).unwrap_or("")))
                            }
                        };
                    }
                    from_json_any(j)
                }
            },
        }
    }

    /// Decode like an existing value (for `pref` without a type annotation).
    pub(crate) fn decode_like(&mut self, j: &J, template: &Value) -> Value {
        match (template, j) {
            (Value::Int(_), J::Number(x)) => Value::Int(x.as_i64().unwrap_or_else(|| x.as_f64().unwrap_or(0.0) as i64)),
            (Value::Float(_), J::Number(x)) => Value::Float(x.as_f64().unwrap_or(0.0)),
            (Value::Str(_), J::String(s)) => Value::str(s),
            (Value::Bool(_), J::Bool(b)) => Value::Bool(*b),
            (Value::Color(_), J::String(s)) => parse_color(s).map(Value::Color).unwrap_or_else(|| template.clone()),
            (Value::Enum(_), J::String(s)) => Value::Enum(Rc::from(s.as_str())),
            (Value::Struct(sv), J::Object(_)) => {
                let fields = sv
                    .fields
                    .iter()
                    .map(|(k, dv)| (k.clone(), j.get(&**k).map(|x| self.decode_like(x, dv)).unwrap_or_else(|| dv.clone())))
                    .collect();
                Value::Struct(Rc::new(StructVal { name: sv.name.clone(), fields }))
            }
            (Value::List(l), J::Array(a)) => {
                let item = l.first().cloned();
                Value::List(Rc::new(a.iter().map(|x| item.as_ref().map(|t| self.decode_like(x, t)).unwrap_or_else(|| from_json_any(x))).collect()))
            }
            (Value::Map(_), J::Object(_)) => from_json_any(j),
            _ => template.clone(),
        }
    }
}
