//! `core/Components.js` against `notchd_core::components`. Catalogs and
//! components are the registry's entries as JSON; functions that return
//! components return their ids (`parseCatalog`: id, title, kind);
//! `parseStatus` returns `{ id: { installed, active } }`; an `undefined`
//! (or null) installed list is `None` (not read yet); `valueOf` is an object.
use super::*;
use notchd_core::components as c;
use std::collections::{BTreeMap, HashMap};

fn component(v: &Value) -> c::Component {
    from_json(v)
}

fn catalog(v: &Value) -> Vec<c::Component> {
    v.as_array()
        .map(|l| l.iter().map(component).collect())
        .unwrap_or_default()
}

fn states(v: &Value) -> BTreeMap<String, c::ComponentState> {
    // The installer's parser drops falsy entries, as the JS rules ignore them.
    if v.is_null() {
        BTreeMap::new()
    } else {
        c::parse_status(&v.to_string())
    }
}

fn titles(v: &Value) -> HashMap<String, String> {
    v.as_object()
        .map(|o| {
            o.iter()
                .map(|(k, x)| (k.clone(), x.as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn installed(a: &[Value], i: usize) -> Option<Vec<String>> {
    opt(a, i).filter(|v| !v.is_null()).map(strs)
}

fn ids(l: Vec<&c::Component>) -> Value {
    json!(l.into_iter().map(|c| c.id.clone()).collect::<Vec<_>>())
}

pub fn call(f: &str, a: &[Value]) -> Option<Value> {
    Some(match f {
        "parseCatalog" => {
            let list = c::parse_catalog(arg(a, 0).as_str().unwrap_or(""));
            json!(
                list.iter()
                    .map(|x| json!({ "id": x.id, "title": x.title, "kind": to_json(&x.kind) }))
                    .collect::<Vec<_>>()
            )
        }
        "parseStatus" => {
            let m = c::parse_status(arg(a, 0).as_str().unwrap_or(""));
            Value::Object(
                m.iter()
                    .map(|(k, s)| {
                        (
                            k.clone(),
                            json!({ "installed": s.installed, "active": s.active }),
                        )
                    })
                    .collect(),
            )
        }
        "installerCommand" => json!(c::installer_command(&jstext(a, 0), &strs(arg(a, 1)))),
        "formatBytes" => {
            let n = match arg(a, 0) {
                Value::Null => 0.0,
                Value::String(t) => t.trim().parse().unwrap_or(f64::NAN),
                Value::Bool(b) => f64::from(u8::from(*b)),
                v => v.as_f64().unwrap_or(f64::NAN),
            };
            json!(c::format_bytes(n))
        }
        "sizeToInstall" => n(c::size_to_install(
            &component(arg(a, 0)),
            arg(a, 1).as_bool().unwrap_or(false),
        )),
        "sharesBackend" => json!(c::shares_backend(
            &component(arg(a, 0)),
            &catalog(arg(a, 1)),
            &installed(a, 2).unwrap_or_default()
        )),
        "view" => {
            let st: Option<c::ComponentState> = opt(a, 1).filter(|v| !v.is_null()).map(from_json);
            to_json(&c::view(
                &component(arg(a, 0)),
                st.as_ref(),
                arg(a, 2).as_bool().unwrap_or(false),
            ))
        }
        "installedIds" => json!(c::installed_ids(&states(arg(a, 0)))),
        "anyActive" => json!(c::any_active(&states(arg(a, 0)))),
        "usedBy" => json!(c::used_by(&component(arg(a, 0)), &titles(arg(a, 1)))),
        "helpText" => json!(c::help_text(&component(arg(a, 0)), &titles(arg(a, 1)))),
        "field" => c::field(&text(a, 0), &text(a, 1), &text(a, 2)),
        "overviewFields" => json!(c::overview_fields(&catalog(arg(a, 0)), &states(arg(a, 1)))),
        "diskText" => json!(c::disk_text(&catalog(arg(a, 0)), &states(arg(a, 1)))),
        "matching" => {
            let cat = catalog(arg(a, 0));
            let where_ = arg(a, 1).as_object().cloned().unwrap_or_default();
            ids(c::matching(&cat, &where_))
        }
        "expand" => json!(c::expand(arg(a, 0), &catalog(arg(a, 1)))),
        "needed" => {
            let cat = catalog(arg(a, 1));
            let values = arg(a, 2).as_object().cloned();
            let value_of = |k: &str| values.as_ref().and_then(|v| v.get(k).cloned());
            let r = c::needed(
                arg(a, 0),
                &cat,
                values.as_ref().map(|_| &value_of as c::ValueOf),
                arg(a, 3).as_bool().unwrap_or(false),
            );
            or_null(r.map(|l| json!(l)))
        }
        "decorate" => json!(c::decorate(
            &list(a, 0),
            &catalog(arg(a, 1)),
            installed(a, 2).as_deref(),
            &titles(arg(a, 3))
        )),
        "gate" => {
            let values = arg(a, 3).as_object().cloned().unwrap_or_default();
            json!(c::gate(
                &list(a, 0),
                &catalog(arg(a, 1)),
                installed(a, 2).as_deref(),
                &|k: &str| values.get(k).cloned()
            ))
        }
        "missingHint" => json!(c::missing_hint(&strs(arg(a, 0)), &catalog(arg(a, 1)))),
        _ => return None,
    })
}
