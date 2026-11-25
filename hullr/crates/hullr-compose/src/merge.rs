//! Merging of multiple compose files at the YAML value level.
//!
//! Later files win. Mappings merge recursively; sequences under list-like
//! keys (ports, volumes, ...) append with de-duplication; everything else
//! (including `command` and `entrypoint`) is replaced.

use serde_yaml::{Mapping, Value};

/// Keys whose sequence values are appended rather than replaced.
const APPEND_KEYS: &[&str] = &[
    "ports", "volumes", "env_file", "expose", "extra_hosts", "profiles", "environment", "labels",
    "args", "tmpfs", "depends_on", "dns", "cap_add", "cap_drop",
];

/// Merge `over` on top of `base`.
pub fn merge(base: Value, over: Value) -> Value {
    merge_value("", base, over)
}

fn merge_value(key: &str, base: Value, over: Value) -> Value {
    let (base, over) = reconcile(key, base, over);
    match (base, over) {
        (Value::Mapping(a), Value::Mapping(b)) => Value::Mapping(merge_maps(a, b)),
        (Value::Sequence(a), Value::Sequence(b)) if APPEND_KEYS.contains(&key) => {
            Value::Sequence(append_dedupe(key, a, b))
        }
        (_, over) => over,
    }
}

fn merge_maps(mut a: Mapping, b: Mapping) -> Mapping {
    for (k, v) in b {
        let name = k.as_str().unwrap_or("").to_string();
        match a.remove(&k) {
            Some(old) => {
                a.insert(k, merge_value(&name, old, v));
            }
            None => {
                a.insert(k, v);
            }
        }
    }
    a
}

/// Make shapes compatible when one file uses list form and the other map form.
fn reconcile(key: &str, a: Value, b: Value) -> (Value, Value) {
    match (&a, &b) {
        (Value::Sequence(_), Value::Mapping(_)) | (Value::Mapping(_), Value::Sequence(_))
            if matches!(key, "environment" | "labels" | "args" | "depends_on") =>
        {
            (to_map(key, a), to_map(key, b))
        }
        (Value::String(s), Value::Mapping(_)) if key == "build" => (ctx_map(s), b),
        (Value::Mapping(_), Value::String(s)) if key == "build" => {
            let m = ctx_map(s);
            (a, m)
        }
        _ => (a, b),
    }
}

fn ctx_map(s: &str) -> Value {
    let mut m = Mapping::new();
    m.insert("context".into(), Value::String(s.to_string()));
    Value::Mapping(m)
}

fn to_map(key: &str, v: Value) -> Value {
    let Value::Sequence(seq) = v else { return v };
    let mut m = Mapping::new();
    for item in seq {
        let Some(s) = item.as_str() else { continue };
        if key == "depends_on" {
            let mut d = Mapping::new();
            d.insert("condition".into(), "service_started".into());
            m.insert(s.into(), Value::Mapping(d));
        } else if let Some((k, val)) = s.split_once('=') {
            m.insert(k.into(), Value::String(val.to_string()));
        } else {
            m.insert(s.into(), Value::Null);
        }
    }
    Value::Mapping(m)
}

/// Identity of a sequence item for de-duplication. Volumes collide on their
/// container path, everything else on the whole item.
fn identity(key: &str, v: &Value) -> String {
    if key == "volumes" {
        match v {
            Value::String(s) => {
                let parts: Vec<&str> = s.split(':').collect();
                return match parts.as_slice() {
                    [only] => only.to_string(),
                    [_, dst, ..] => dst.to_string(),
                    _ => s.clone(),
                };
            }
            Value::Mapping(m) => {
                if let Some(t) = m.get("target").and_then(Value::as_str) {
                    return t.to_string();
                }
            }
            _ => {}
        }
    }
    serde_yaml::to_string(v).unwrap_or_default()
}

fn append_dedupe(key: &str, a: Vec<Value>, b: Vec<Value>) -> Vec<Value> {
    let mut out = a;
    for item in b {
        let id = identity(key, &item);
        if let Some(pos) = out.iter().position(|x| identity(key, x) == id) {
            if key == "volumes" {
                out[pos] = item;
            }
        } else {
            out.push(item);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn y(s: &str) -> Value {
        serde_yaml::from_str(s).unwrap()
    }

    #[test]
    fn maps_merge_and_scalars_replace() {
        let m = merge(
            y("services: {a: {image: x, restart: no}}"),
            y("services: {a: {restart: always}, b: {image: y}}"),
        );
        assert_eq!(m, y("services: {a: {image: x, restart: always}, b: {image: y}}"));
    }

    #[test]
    fn sequences_append_and_command_replaces() {
        let m = merge(
            y("services: {a: {ports: ['80:80'], command: [a, b], profiles: [x]}}"),
            y("services: {a: {ports: ['80:80', '81:81'], command: [c], profiles: [y]}}"),
        );
        assert_eq!(
            m,
            y("services: {a: {ports: ['80:80', '81:81'], command: [c], profiles: [x, y]}}")
        );
    }

    #[test]
    fn volumes_dedupe_by_target() {
        let m = merge(
            y("services: {a: {volumes: ['d1:/data', 'x:/x']}}"),
            y("services: {a: {volumes: ['d2:/data']}}"),
        );
        assert_eq!(m, y("services: {a: {volumes: ['d2:/data', 'x:/x']}}"));
    }

    #[test]
    fn env_list_and_map_mix() {
        let m = merge(
            y("services: {a: {environment: [A=1, B=2]}}"),
            y("services: {a: {environment: {B: '3', C: null}}}"),
        );
        assert_eq!(m, y("services: {a: {environment: {A: '1', B: '3', C: null}}}"));
    }

    #[test]
    fn depends_on_list_and_map() {
        let m = merge(
            y("services: {a: {depends_on: [db]}}"),
            y("services: {a: {depends_on: {db: {condition: service_healthy}}}}"),
        );
        assert_eq!(
            m,
            y("services: {a: {depends_on: {db: {condition: service_healthy}}}}")
        );
    }

    #[test]
    fn build_string_and_map() {
        let m = merge(
            y("services: {a: {build: ./app}}"),
            y("services: {a: {build: {target: dev}}}"),
        );
        assert_eq!(m, y("services: {a: {build: {context: ./app, target: dev}}}"));
    }
}
