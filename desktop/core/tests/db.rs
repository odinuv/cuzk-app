use chrono::{DateTime, Duration, TimeZone, Utc};
use strazce_core::db::{Db, NewItem, RunRecord};
use strazce_core::model::{DruhCislovani, ItemIdent, ParcelaRef};

fn ident(k: u32) -> ItemIdent {
    ItemIdent::Parcela {
        ku: 618853,
        parcela: ParcelaRef { druh: DruhCislovani::Stavebni, kmenove: k, poddeleni: None },
    }
}

fn new_item(k: u32, enabled: bool) -> NewItem {
    NewItem { popis: format!("p{k}"), enabled, ident: ident(k) }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 17, 8, 0, 0).unwrap()
}

fn finished(id: i64) -> RunRecord {
    RunRecord {
        id,
        finished_at: now(),
        status: "ok".into(),
        checked: 1,
        changed: 1,
        errors: 0,
        deferred: 0,
        api_calls: 2,
        api_used_today: Some(10),
        api_limit: Some(500),
        aktualnost_dat: None,
        error: None,
    }
}

#[test]
fn migrate_twice_is_noop() {
    let dir = std::env::temp_dir().join(format!("strazce-db-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.db");
    {
        let db = Db::open(&path).unwrap();
        db.insert_item(&new_item(1, true)).unwrap();
    }
    let db = Db::open(&path).unwrap();
    assert_eq!(db.list_items().unwrap().len(), 1);
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn due_order_never_checked_first_then_oldest() {
    let db = Db::open_in_memory().unwrap();
    let a = db.insert_item(&new_item(1, true)).unwrap();
    let b = db.insert_item(&new_item(2, true)).unwrap();
    let c = db.insert_item(&new_item(3, true)).unwrap();
    db.set_item_success(a, "{}", now() - Duration::days(20)).unwrap();
    db.set_item_success(b, "{}", now() - Duration::days(30)).unwrap();
    let ids: Vec<i64> = db.due_items(now(), 14).unwrap().iter().map(|r| r.item.id).collect();
    assert_eq!(ids, vec![c, b, a]);
}

#[test]
fn due_excludes_disabled_and_fresh() {
    let db = Db::open_in_memory().unwrap();
    let fresh = db.insert_item(&new_item(1, true)).unwrap();
    let edge = db.insert_item(&new_item(2, true)).unwrap();
    let off = db.insert_item(&new_item(3, false)).unwrap();
    db.set_item_success(fresh, "{}", now() - Duration::days(13)).unwrap();
    db.set_item_success(edge, "{}", now() - Duration::days(14)).unwrap();
    let ids: Vec<i64> = db.due_items(now(), 14).unwrap().iter().map(|r| r.item.id).collect();
    assert_eq!(ids, vec![edge]);
    assert!(!ids.contains(&off));
}

#[test]
fn identity_change_resets_state_keeps_history() {
    let db = Db::open_in_memory().unwrap();
    let id = db.insert_item(&new_item(1, true)).unwrap();
    let key = strazce_core::model::item_key(&ident(1));
    db.set_resolved(id, &strazce_core::db::Resolved { item_key: key, endpoint: "parcely".into(), iskn_id: 5 }).unwrap();
    db.set_item_success(id, "{\"a\":1}", now()).unwrap();
    db.set_resolve_error(id, Some("x".into())).unwrap();
    let run = db.begin_run("manual", now()).unwrap();
    db.insert_item_result(run, id, "first", None, None).unwrap();

    // popis-only change keeps state
    let mut item = db.list_items().unwrap().remove(0).item;
    item.popis = "nové".into();
    db.update_item(&item).unwrap();
    assert!(db.get_resolved(id).unwrap().is_some());
    assert!(db.list_items().unwrap()[0].last_data.is_some());

    item.ident = ident(2);
    db.update_item(&item).unwrap();
    let row = db.list_items().unwrap().remove(0);
    assert!(db.get_resolved(id).unwrap().is_none());
    assert!(row.last_data.is_none());
    assert!(row.last_success_at.is_none());
    assert!(row.resolve_error.is_none());
    assert_eq!(db.due_items(now(), 14).unwrap().len(), 1);
    db.finish_run(&finished(run)).unwrap();
    assert_eq!(db.recent_runs(10).unwrap().len(), 1);
    assert_eq!(db.count_item_results().unwrap(), 1);
}

#[test]
fn delete_item_cascades_results() {
    let db = Db::open_in_memory().unwrap();
    let id = db.insert_item(&new_item(1, true)).unwrap();
    let run = db.begin_run("manual", now()).unwrap();
    db.insert_item_result(run, id, "changed", Some("[]"), None).unwrap();
    db.delete_item(id).unwrap();
    assert_eq!(db.count_item_results().unwrap(), 0);
    assert!(db.list_items().unwrap().is_empty());
}

#[test]
fn search_ku_ignores_diacritics() {
    let db = Db::open_in_memory().unwrap();
    db.replace_obce(&[(1, "Brno".to_string())], now()).unwrap();
    db.replace_ku(
        &[
            (10, "Zábrdovice".to_string(), 1),
            (11, "Horní Zábrdí".to_string(), 1),
            (12, "Other".to_string(), 1),
        ],
        now(),
    )
    .unwrap();
    let r = db.search_ku("zabrd", 10).unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].nazev, "Zábrdovice");
    assert_eq!(r[0].obec, "Brno");
    assert_eq!(db.search_ku("ZÁBRDOVICE", 10).unwrap().len(), 1);
    assert_eq!(db.ciselnik_age("ku").unwrap(), Some(now()));
    assert_eq!(db.ciselnik_age("obce").unwrap(), Some(now()));
    assert_eq!(db.ciselnik_age("casti_obci").unwrap(), None);
}

#[test]
fn unseen_findings_then_mark_seen() {
    let db = Db::open_in_memory().unwrap();
    let id = db.insert_item(&new_item(1, true)).unwrap();
    let run = db.begin_run("manual", now()).unwrap();
    db.insert_item_result(run, id, "first", None, None).unwrap();
    db.insert_item_result(run, id, "unchanged", None, None).unwrap();
    db.insert_item_result(run, id, "changed", Some("[{\"x\":1}]"), None).unwrap();
    db.finish_run(&finished(run)).unwrap();
    let f = db.unseen_findings().unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].status, "changed");
    assert_eq!(f[0].popis, "p1");
    db.mark_all_seen().unwrap();
    assert!(db.unseen_findings().unwrap().is_empty());
    assert_eq!(db.get_setting("k").unwrap(), None);
    db.set_setting("k", "v").unwrap();
    db.set_setting("k", "w").unwrap();
    assert_eq!(db.get_setting("k").unwrap(), Some("w".into()));
}
