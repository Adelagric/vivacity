//! PHP array operations on JSON values, as the merge plugin and Composer
//! apply them to manifest sections: a JSON object is a string-keyed PHP
//! array, a JSON array an integer-keyed one.

use serde_json::{Map, Value};

/// `array_merge($a, $b)`: string keys of `b` overwrite `a`'s (in `a`'s
/// position, new ones appended); integer-keyed entries are appended and
/// renumbered. Two lists concatenate; a list and an object cannot meet in
/// the schema, the object wins then.
pub fn array_merge(a: &Value, b: &Value) -> Value {
    match (a, b) {
        (Value::Object(oa), Value::Object(ob)) => {
            let mut out = oa.clone();
            for (k, v) in ob {
                out.insert(k.clone(), v.clone());
            }
            Value::Object(out)
        }
        (Value::Array(la), Value::Array(lb)) => {
            let mut out = la.clone();
            out.extend(lb.iter().cloned());
            Value::Array(out)
        }
        (Value::Object(_), _) => a.clone(),
        (_, other) => other.clone(),
    }
}

/// `array_merge_recursive($a, $b)`: a string key present in both merges
/// recursively when both values are arrays, otherwise the two values
/// become a list (`[a, b]`; a list on one side takes the other side's
/// value appended, in order); integer keys append.
pub fn array_merge_recursive(a: &Value, b: &Value) -> Value {
    match (a, b) {
        (Value::Object(oa), Value::Object(ob)) => {
            let mut out: Map<String, Value> = oa.clone();
            for (k, vb) in ob {
                match out.get(k).cloned() {
                    None => {
                        out.insert(k.clone(), vb.clone());
                    }
                    Some(va) => {
                        let merged = match (&va, vb) {
                            (Value::Object(_), Value::Object(_))
                            | (Value::Array(_), Value::Array(_)) => array_merge_recursive(&va, vb),
                            (Value::Array(la), scalar) => {
                                let mut l = la.clone();
                                l.push(scalar.clone());
                                Value::Array(l)
                            }
                            (scalar, Value::Array(lb)) => {
                                let mut l = vec![scalar.clone()];
                                l.extend(lb.iter().cloned());
                                Value::Array(l)
                            }
                            (sa, sb) => Value::Array(vec![sa.clone(), sb.clone()]),
                        };
                        out.insert(k.clone(), merged);
                    }
                }
            }
            Value::Object(out)
        }
        (Value::Array(la), Value::Array(lb)) => {
            let mut out = la.clone();
            out.extend(lb.iter().cloned());
            Value::Array(out)
        }
        (Value::Array(la), other) => {
            let mut out = la.clone();
            out.push(other.clone());
            Value::Array(out)
        }
        (other, Value::Array(lb)) => {
            let mut out = vec![other.clone()];
            out.extend(lb.iter().cloned());
            Value::Array(out)
        }
        (x, y) => Value::Array(vec![x.clone(), y.clone()]),
    }
}

/// `NestedArray::mergeDeepArray([$a, $b])` of the merge plugin: integer
/// keys append, a string key whose values are both arrays merges
/// recursively, otherwise the later value wins.
pub fn merge_deep(a: &Value, b: &Value) -> Value {
    match (a, b) {
        (Value::Object(oa), Value::Object(ob)) => {
            let mut out = oa.clone();
            for (k, vb) in ob {
                let merged = match out.get(k) {
                    Some(va)
                        if matches!(
                            (va, vb),
                            (Value::Object(_), Value::Object(_))
                                | (Value::Array(_), Value::Array(_))
                                | (Value::Object(_), Value::Array(_))
                                | (Value::Array(_), Value::Object(_))
                        ) =>
                    {
                        merge_deep(va, vb)
                    }
                    _ => vb.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        (Value::Array(la), Value::Array(lb)) => {
            let mut out = la.clone();
            out.extend(lb.iter().cloned());
            Value::Array(out)
        }
        // A PHP array is one thing: a list met by a map appends the list's
        // items (integer keys) into the map's entries — the schema never
        // does this; the second value wins.
        (_, other) => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_recursive_like_php() {
        // psr-4: same key twice → list; three times → the list grows.
        let root = json!({"psr-4": {"App\\": "src/"}, "files": ["a.php"]});
        let inc = json!({"psr-4": {"App\\": "modules/x/src/", "Mod\\": "modules/x/lib/"}, "files": ["modules/x/b.php"], "exclude-from-classmap": ["modules/x/tests/"]});
        let m = array_merge_recursive(&root, &inc);
        assert_eq!(
            m,
            json!({"psr-4": {"App\\": ["src/", "modules/x/src/"], "Mod\\": "modules/x/lib/"}, "files": ["a.php", "modules/x/b.php"], "exclude-from-classmap": ["modules/x/tests/"]})
        );
        let inc2 = json!({"psr-4": {"App\\": ["y/", "z/"]}});
        let m2 = array_merge_recursive(&m, &inc2);
        assert_eq!(
            m2["psr-4"]["App\\"],
            json!(["src/", "modules/x/src/", "y/", "z/"])
        );
        // scalar met by a list: scalar first.
        assert_eq!(
            array_merge_recursive(&json!({"k": "a"}), &json!({"k": ["b", "c"]})),
            json!({"k": ["a", "b", "c"]})
        );
    }

    #[test]
    fn merge_and_deep_like_php() {
        assert_eq!(
            array_merge(
                &json!({"a": 1, "b": {"x": 1}}),
                &json!({"b": {"y": 2}, "c": 3})
            ),
            json!({"a": 1, "b": {"y": 2}, "c": 3})
        );
        assert_eq!(
            merge_deep(
                &json!({"a": 1, "b": {"x": 1, "l": [1]}}),
                &json!({"b": {"y": 2, "l": [2]}, "a": 9})
            ),
            json!({"a": 9, "b": {"x": 1, "l": [1, 2], "y": 2}})
        );
    }
}
