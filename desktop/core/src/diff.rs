//! Normalization and diffing of cadastre object JSON.

use crate::api::RizeniDef;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldChange {
    pub path: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

/// Sorts object keys; arrays are normalized element-wise and ordered by their
/// serialized form (scalars too, like the PoC's diff.ts).
pub fn normalize(v: &Value) -> Value {
    match v {
        Value::Array(items) => {
            let mut keyed: Vec<(String, Value)> = items
                .iter()
                .map(normalize)
                .map(|n| (n.to_string(), n))
                .collect();
            keyed.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Array(keyed.into_iter().map(|(_, n)| n).collect())
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for k in keys {
                out.insert(k.clone(), normalize(&map[k]));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn collect(before: &Map<String, Value>, after: &Map<String, Value>, prefix: &str, top: bool, out: &mut Vec<FieldChange>) {
    let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for key in keys {
        // Plomby are diffed separately; the ISKN `id` is a lookup artifact (identity is
        // fixed by the item's identification), so a re-resolved id is not a change.
        if top && (key == "rizeniPlomby" || key == "id") {
            continue;
        }
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match (before.get(key), after.get(key)) {
            (Some(Value::Object(b)), Some(Value::Object(a))) => collect(b, a, &path, false, out),
            (b, a) => {
                if b != a {
                    out.push(FieldChange { path, before: b.cloned(), after: a.cloned() });
                }
            }
        }
    }
}

/// Field-level diff of two cadastre objects (both normalized first).
/// Top-level `rizeniPlomby` and `id` are excluded; output is sorted by path.
pub fn diff_object(before: &Value, after: &Value) -> Vec<FieldChange> {
    let empty = Map::new();
    let (nb, na) = (normalize(before), normalize(after));
    let b = nb.as_object().unwrap_or(&empty);
    let a = na.as_object().unwrap_or(&empty);
    let mut out = Vec::new();
    collect(b, a, "", true, &mut out);
    out.sort_by(|x, y| x.path.cmp(&y.path));
    out
}

/// Returns (new, gone) liens matched by `id`, preserving input order.
pub fn diff_plomby(before: &[RizeniDef], after: &[RizeniDef]) -> (Vec<RizeniDef>, Vec<RizeniDef>) {
    let bi: HashSet<i64> = before.iter().map(|r| r.id).collect();
    let ai: HashSet<i64> = after.iter().map(|r| r.id).collect();
    (
        after.iter().filter(|r| !bi.contains(&r.id)).cloned().collect(),
        before.iter().filter(|r| !ai.contains(&r.id)).cloned().collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rd(id: i64) -> RizeniDef {
        RizeniDef { id, typ_rizeni: "V".into(), poradove_cislo: 1, rok: 2026, kod_pracoviste: 701 }
    }

    #[test]
    fn reordered_array_is_no_change() {
        let a = json!({"vlastnici": [{"id": 1, "x": "a"}, {"id": 2, "x": "b"}], "tags": [3, 1, 2]});
        let b = json!({"vlastnici": [{"x": "b", "id": 2}, {"id": 1, "x": "a"}], "tags": [1, 2, 3]});
        assert!(diff_object(&a, &b).is_empty());
    }

    #[test]
    fn nested_field_change_has_dot_path() {
        let a = json!({"zpusobVyuziti": {"nazev": "orná půda", "kod": 2}});
        let b = json!({"zpusobVyuziti": {"nazev": "zahrada", "kod": 2}});
        let d = diff_object(&a, &b);
        assert_eq!(d, vec![FieldChange {
            path: "zpusobVyuziti.nazev".into(),
            before: Some(json!("orná půda")),
            after: Some(json!("zahrada")),
        }]);
    }

    #[test]
    fn added_and_removed_fields() {
        let a = json!({"old": 1, "same": true});
        let b = json!({"new": 2, "same": true});
        let d = diff_object(&a, &b);
        assert_eq!(d, vec![
            FieldChange { path: "new".into(), before: None, after: Some(json!(2)) },
            FieldChange { path: "old".into(), before: Some(json!(1)), after: None },
        ]);
    }

    #[test]
    fn plomby_not_in_field_diff() {
        let a = json!({"rizeniPlomby": [{"id": 1}], "x": {"rizeniPlomby": 1}});
        let b = json!({"rizeniPlomby": [{"id": 2}], "x": {"rizeniPlomby": 2}});
        let d = diff_object(&a, &b);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].path, "x.rizeniPlomby");
    }

    #[test]
    fn top_level_id_not_in_field_diff() {
        let a = json!({"id": 101, "vymera": 5, "x": {"id": 1}});
        let b = json!({"id": 202, "vymera": 5, "x": {"id": 2}});
        let d = diff_object(&a, &b);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].path, "x.id");
    }

    #[test]
    fn plomby_new_and_gone_by_id() {
        let (nove, zanikle) = diff_plomby(&[rd(3), rd(1), rd(2)], &[rd(5), rd(2), rd(4)]);
        assert_eq!(nove, vec![rd(5), rd(4)]);
        assert_eq!(zanikle, vec![rd(3), rd(1)]);
    }
}
