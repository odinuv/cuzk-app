use chrono::{DateTime, Duration as Dur, TimeZone, Utc};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use strazce_core::api::{ApiClient, MockTransport, Transport};
use strazce_core::db::{Db, ItemRow, NewItem};
use strazce_core::error::CoreError;
use strazce_core::lock::RunLock;
use strazce_core::model::*;
use strazce_core::run::{run_check, Changes, RunStatus, Trigger};
use tempfile::TempDir;

struct Env {
    mock: Arc<MockTransport>,
    client: ApiClient,
    db: Db,
    _dir: TempDir,
    lock: PathBuf,
}

fn setup() -> Env {
    let mock = Arc::new(MockTransport::new());
    let client = ApiClient::new(mock.clone(), "fake-key".into()).with_retry_delay(Duration::ZERO);
    let dir = tempfile::tempdir().unwrap();
    // Lock lives in a not-yet-existing subdirectory: acquire must create it.
    let lock = dir.path().join("data").join("strazce.lock");
    Env {
        mock,
        client,
        db: Db::open_in_memory().unwrap(),
        _dir: dir,
        lock,
    }
}

impl Env {
    fn run(&mut self, now: DateTime<Utc>) -> Result<strazce_core::run::RunSummary, CoreError> {
        run_check(&self.client, &mut self.db, &self.lock, Trigger::Manual, now)
    }

    fn stav(&self, provedeno: u32, limit: u32) {
        self.mock.push(
            "/AplikacniSluzby/StavUctu",
            200,
            &format!(
                r#"{{"idSluzby":1,"aktualniObdobi":"2026-11-17","provedenoVolani":{provedeno},"limitVolani":{limit},"expiraceApiKey":null}}"#
            ),
        );
    }

    fn item(&self, id: i64) -> ItemRow {
        self.db
            .list_items()
            .unwrap()
            .into_iter()
            .find(|r| r.item.id == id)
            .unwrap()
    }

    fn statuses(&self, run_id: i64) -> Vec<(i64, String)> {
        self.db
            .item_results_for_run(run_id)
            .unwrap()
            .into_iter()
            .map(|r| (r.item_id, r.status))
            .collect()
    }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 17, 8, 0, 0).unwrap()
}

fn parcela(k: u32) -> ItemIdent {
    ItemIdent::Parcela {
        ku: 618853,
        parcela: ParcelaRef {
            druh: DruhCislovani::Stavebni,
            kmenove: k,
            poddeleni: None,
        },
    }
}

fn add(db: &Db, ident: ItemIdent) -> i64 {
    db.insert_item(&NewItem {
        popis: "t".into(),
        enabled: true,
        ident,
    })
    .unwrap()
}

fn vyhledani(mock: &MockTransport, id: i64) {
    mock.push(
        "/Parcely/Vyhledani",
        200,
        &format!(r#"{{"data":[{{"id":{id}}}]}}"#),
    );
}

fn plomba(id: i64) -> String {
    format!(r#"{{"id":{id},"typRizeni":"V","poradoveCislo":{id},"rok":2026,"kodPracoviste":701}}"#)
}

fn parcela_body(id: i64, vymera: u32, plomby: &[i64]) -> String {
    let pl: Vec<String> = plomby.iter().map(|p| plomba(*p)).collect();
    format!(
        r#"{{"data":{{"id":{id},"vymera":{vymera},"rizeniPlomby":[{}]}},"aktualnostDatK":"2026-11-16T23:00:00"}}"#,
        pl.join(",")
    )
}

fn get_parcela(mock: &MockTransport, id: i64, vymera: u32, plomby: &[i64]) {
    mock.push("/Parcely/", 200, &parcela_body(id, vymera, plomby));
}

#[test]
fn first_run_marks_first_and_stores_data() {
    let mut e = setup();
    let id = add(&e.db, parcela(182));
    e.stav(10, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 500, &[]);

    let s = e.run(now()).unwrap();
    assert_eq!(s.status, RunStatus::Ok);
    assert_eq!(
        (s.checked, s.changed, s.plomby_nove, s.errors, s.deferred),
        (1, 0, 0, 0, 0)
    );
    assert_eq!(s.api_calls, 3);
    assert_eq!(s.api_used_today, 13);
    assert_eq!(s.api_limit, 500);
    assert_eq!(s.aktualnost_dat.as_deref(), Some("2026-11-16T23:00:00"));
    assert!(s.error.is_none());

    let row = e.item(id);
    let data: serde_json::Value = serde_json::from_str(row.last_data.as_deref().unwrap()).unwrap();
    assert_eq!(data["vymera"], 500);
    assert_eq!(row.last_success_at, Some(now()));
    assert_eq!(e.statuses(s.run_id), vec![(id, "first".to_string())]);
    // A first check without plomby is not a finding.
    assert!(e.db.unseen_findings().unwrap().is_empty());
    let runs = e.db.recent_runs(5).unwrap();
    assert_eq!(runs[0].id, s.run_id);
    assert_eq!(runs[0].status, "ok");
    assert_eq!(runs[0].trigger, "manual");
    assert_eq!(runs[0].finished_at, Some(now()));
    assert_eq!(runs[0].api_used_today, Some(13));
}

#[test]
fn second_run_unchanged_no_findings() {
    let mut e = setup();
    let id = add(&e.db, parcela(182));
    e.stav(10, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 500, &[7]);
    e.mock.push("/Rizeni/", 400, "bad"); // first check reports the plomba (no detail)
    e.run(now()).unwrap();

    let later = now() + Dur::days(15);
    e.stav(0, 500);
    get_parcela(&e.mock, 101, 500, &[7]);
    let s = e.run(later).unwrap();
    assert_eq!(s.status, RunStatus::Ok);
    assert_eq!((s.checked, s.changed, s.plomby_nove), (1, 0, 0));
    // Cached ISKN id: StavUctu + GET only.
    assert_eq!(s.api_calls, 2);
    assert_eq!(e.statuses(s.run_id), vec![(id, "unchanged".to_string())]);
    assert!(e.db.item_results_for_run(s.run_id).unwrap()[0]
        .changes_json
        .is_none());
    assert!(e
        .db
        .unseen_findings()
        .unwrap()
        .iter()
        .all(|f| f.run_id != s.run_id));
    assert_eq!(e.item(id).last_success_at, Some(later));
}

#[test]
fn field_change_and_new_plomba_reported_with_detail() {
    let mut e = setup();
    let a = add(&e.db, parcela(1));
    let b = add(&e.db, parcela(2));
    e.stav(10, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 500, &[1]);
    e.mock.push("/Rizeni/", 400, "bad");
    vyhledani(&e.mock, 102);
    get_parcela(&e.mock, 102, 300, &[]);
    let first = e.run(now()).unwrap();

    let later = now() + Dur::days(15);
    e.stav(0, 500);
    // a: field change + plomba 1 gone + plomba 2 new (detail OK)
    get_parcela(&e.mock, 101, 450, &[2]);
    e.mock.push(
        "/Rizeni/",
        200,
        r#"{"data":{"id":2,"datumPrijeti":"2026-11-10T09:00:00","stav":"probíhá","provedeneOperace":[{"nazev":"Zahájení řízení"},{"nazev":"Vyznačení plomby","datumProvedeni":"2026-11-10"}]}}"#,
    );
    // b: only a new plomba 3 whose detail fails -> still reported, without detail
    get_parcela(&e.mock, 102, 300, &[3]);
    e.mock.push("/Rizeni/", 400, "bad");

    let s = e.run(later).unwrap();
    assert_eq!(s.status, RunStatus::Ok);
    assert_eq!(
        (s.checked, s.changed, s.plomby_nove, s.errors),
        (2, 2, 2, 0)
    );
    assert_eq!(
        e.statuses(s.run_id),
        vec![(a, "changed".into()), (b, "changed".into())]
    );

    let findings: Vec<_> =
        e.db.unseen_findings()
            .unwrap()
            .into_iter()
            .filter(|f| f.run_id != first.run_id)
            .collect();
    assert_eq!(findings.len(), 2);
    let ca: Changes = serde_json::from_str(
        findings
            .iter()
            .find(|f| f.item_id == a)
            .unwrap()
            .changes_json
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(ca.fields.len(), 1);
    assert_eq!(ca.fields[0].path, "vymera");
    assert_eq!(ca.fields[0].before, Some(serde_json::json!(500)));
    assert_eq!(ca.fields[0].after, Some(serde_json::json!(450)));
    assert_eq!(ca.plomby_nove.len(), 1);
    let d = &ca.plomby_nove[0];
    assert_eq!(d.rizeni.id, 2);
    assert_eq!(d.rizeni.label(), "V-2/2026-701");
    assert_eq!(d.datum_prijeti.as_deref(), Some("2026-11-10T09:00:00"));
    assert_eq!(d.stav.as_deref(), Some("probíhá"));
    assert_eq!(
        d.operace,
        vec![
            "Zahájení řízení".to_string(),
            "Vyznačení plomby".to_string()
        ]
    );
    assert_eq!(
        ca.plomby_zanikle.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![1]
    );

    let cb: Changes = serde_json::from_str(
        findings
            .iter()
            .find(|f| f.item_id == b)
            .unwrap()
            .changes_json
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    assert!(cb.fields.is_empty());
    assert_eq!(cb.plomby_nove.len(), 1);
    assert_eq!(cb.plomby_nove[0].rizeni.id, 3);
    assert!(cb.plomby_nove[0].datum_prijeti.is_none());
    assert!(cb.plomby_nove[0].stav.is_none());
    assert!(cb.plomby_nove[0].operace.is_empty());

    let data: serde_json::Value =
        serde_json::from_str(e.item(a).last_data.as_deref().unwrap()).unwrap();
    assert_eq!(data["vymera"], 450);
}

#[test]
fn api_error_keeps_old_state_and_item_stays_due() {
    let mut e = setup();
    let id = add(&e.db, parcela(182));
    e.stav(10, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 500, &[1]);
    e.mock.push("/Rizeni/", 400, "bad");
    let first = e.run(now()).unwrap();
    let before = e.item(id);

    let later = now() + Dur::days(15);
    e.stav(0, 500);
    for _ in 0..3 {
        e.mock.push("/Parcely/", 503, "Service Unavailable");
    }
    let s = e.run(later).unwrap();
    assert_eq!(s.status, RunStatus::Ok);
    assert_eq!((s.checked, s.changed, s.errors), (0, 0, 1));
    assert_eq!(s.api_calls, 4); // StavUctu + 3 attempts
    let res = e.db.item_results_for_run(s.run_id).unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].status, "error");
    // Only the status: response bodies (possibly HTML) are not stored.
    assert_eq!(res[0].error.as_deref(), Some("HTTP chyba 503"));
    assert!(res[0].changes_json.is_none());
    // No false change, old state intact, still due.
    assert!(e
        .db
        .unseen_findings()
        .unwrap()
        .iter()
        .all(|f| f.run_id == first.run_id));
    let after = e.item(id);
    assert_eq!(after.last_data, before.last_data);
    assert_eq!(after.last_success_at, Some(now()));
    let due: Vec<i64> =
        e.db.due_items(later, 14)
            .unwrap()
            .iter()
            .map(|r| r.item.id)
            .collect();
    assert_eq!(due, vec![id]);

    // Retried later the same day: compared against the kept old state -> unchanged.
    let retry = later + Dur::hours(2);
    e.stav(4, 500);
    get_parcela(&e.mock, 101, 500, &[1]);
    let s = e.run(retry).unwrap();
    assert_eq!(e.statuses(s.run_id), vec![(id, "unchanged".to_string())]);
    assert_eq!(e.item(id).last_success_at, Some(retry));
}

#[test]
fn budget_exhaustion_defers_rest_and_next_run_continues() {
    let mut e = setup();
    let ids: Vec<i64> = (1..=7).map(|k| add(&e.db, parcela(k))).collect();
    // budget = 500 - 480 - 1 - 5 = 14; each first check costs 2 calls (Vyhledani + GET).
    // used: 1 (StavUctu) -> items start at used 1,3,5,7,9 (remaining 13,11,9,7,5 >= 4),
    // the 6th would start at used 11 (remaining 3 < 4) -> deferred with the 7th.
    e.stav(480, 500);
    for k in 1..=5 {
        vyhledani(&e.mock, 100 + k);
        get_parcela(&e.mock, 100 + k, 10, &[]);
    }
    let s = e.run(now()).unwrap();
    assert_eq!(s.status, RunStatus::Partial);
    assert_eq!((s.checked, s.deferred, s.errors), (5, 2, 0));
    assert_eq!(s.api_calls, 11);
    assert_eq!(s.api_used_today, 491);
    assert!(s.api_used_today <= s.api_limit - 5);
    let st = e.statuses(s.run_id);
    assert_eq!(st.len(), 7);
    for (i, (item, status)) in st.iter().enumerate() {
        assert_eq!(*item, ids[i]);
        assert_eq!(status, if i < 5 { "first" } else { "deferred" });
    }
    assert_eq!(e.db.recent_runs(1).unwrap()[0].status, "partial");
    assert!(e.item(ids[5]).last_data.is_none());
    assert!(e.item(ids[5]).last_success_at.is_none());

    // Next day: only the deferred ones are due, and they go first.
    let next = now() + Dur::days(1);
    let urls_before = e.mock.requested_urls().len();
    e.stav(0, 500);
    for k in 6..=7 {
        vyhledani(&e.mock, 100 + k);
        get_parcela(&e.mock, 100 + k, 10, &[]);
    }
    let s = e.run(next).unwrap();
    assert_eq!(s.status, RunStatus::Ok);
    assert_eq!((s.checked, s.deferred), (2, 0));
    assert_eq!(
        e.statuses(s.run_id),
        vec![(ids[5], "first".to_string()), (ids[6], "first".to_string())]
    );
    let urls = &e.mock.requested_urls()[urls_before..];
    assert!(urls[1].contains("KmenoveCisloParcely=6"), "{urls:?}");
    assert!(urls[3].contains("KmenoveCisloParcely=7"), "{urls:?}");
}

#[test]
fn budget_already_exhausted_defers_everything() {
    let mut e = setup();
    let id = add(&e.db, parcela(1));
    // budget = 500 - 495 - 6 = -1: no item fits, no underflow.
    e.stav(495, 500);
    let s = e.run(now()).unwrap();
    assert_eq!(s.status, RunStatus::Partial);
    assert_eq!((s.checked, s.deferred), (0, 1));
    assert_eq!(s.api_calls, 1);
    assert_eq!(e.statuses(s.run_id), vec![(id, "deferred".to_string())]);
}

#[test]
fn nothing_due_makes_zero_api_calls() {
    let mut e = setup();
    // No items at all.
    let s = e.run(now()).unwrap();
    assert_eq!(s.status, RunStatus::Ok);
    assert_eq!((s.checked, s.api_calls), (0, 0));
    assert_eq!(e.client.calls(), 0);

    // An item checked yesterday with the default 14-day interval.
    add(&e.db, parcela(1));
    e.stav(10, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 1, &[]);
    e.run(now()).unwrap();
    let calls = e.client.calls();
    let s = e.run(now() + Dur::days(1)).unwrap();
    assert_eq!(s.status, RunStatus::Ok);
    assert_eq!((s.checked, s.deferred, s.api_calls), (0, 0, 0));
    assert_eq!(e.client.calls(), calls);
    assert!(e.db.item_results_for_run(s.run_id).unwrap().is_empty());
    let runs = e.db.recent_runs(5).unwrap();
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[0].id, s.run_id);
    assert_eq!(runs[0].status, "ok");
    assert_eq!(runs[0].checked, 0);

    // interval_days from settings is honoured: 1 day -> due again.
    e.db.set_setting("interval_days", "1").unwrap();
    e.stav(0, 500);
    get_parcela(&e.mock, 101, 1, &[]);
    let s = e.run(now() + Dur::days(1)).unwrap();
    assert_eq!(s.checked, 1);
}

#[test]
fn second_concurrent_run_fails_already_running() {
    let mut e = setup();
    let id = add(&e.db, parcela(1));
    let held = RunLock::acquire(&e.lock).unwrap();
    assert_eq!(
        RunLock::acquire(&e.lock).err(),
        Some(CoreError::AlreadyRunning)
    );

    // No mock responses queued: any API call would panic.
    assert_eq!(e.run(now()).err(), Some(CoreError::AlreadyRunning));
    assert_eq!(e.client.calls(), 0);
    assert!(e.db.recent_runs(5).unwrap().is_empty());
    assert_eq!(e.db.count_item_results().unwrap(), 0);
    assert!(e.item(id).last_success_at.is_none());

    // Released on drop.
    drop(held);
    let again = RunLock::acquire(&e.lock).unwrap();
    drop(again);
    e.stav(0, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 1, &[]);
    assert_eq!(e.run(now()).unwrap().checked, 1);
}

#[test]
fn stale_iskn_id_404_re_resolves() {
    let mut e = setup();
    let id = add(&e.db, parcela(182));
    e.stav(10, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 500, &[]);
    e.run(now()).unwrap();

    let later = now() + Dur::days(15);
    e.stav(0, 500);
    e.mock.push("/Parcely/", 404, "");
    vyhledani(&e.mock, 202);
    get_parcela(&e.mock, 202, 500, &[]);
    let s = e.run(later).unwrap();
    assert_eq!(s.api_calls, 4);
    assert_eq!(s.errors, 0);
    // Same data under a new ISKN id: not a change.
    assert_eq!(e.statuses(s.run_id), vec![(id, "unchanged".to_string())]);
    assert!(e.db.item_results_for_run(s.run_id).unwrap()[0]
        .changes_json
        .is_none());
    assert!(e.db.unseen_findings().unwrap().is_empty());
    assert_eq!(e.item(id).last_success_at, Some(later));
    assert_eq!(e.db.get_resolved(id).unwrap().unwrap().iskn_id, 202);
    assert!(e
        .mock
        .requested_urls()
        .last()
        .unwrap()
        .ends_with("/Parcely/202"));
}

fn push_503s(mock: &MockTransport, prefix: &str) {
    mock.push(prefix, 503, "Service Unavailable");
    mock.push(prefix, 503, "Service Unavailable");
}

#[test]
fn retries_never_exceed_budget() {
    let mut e = setup();
    let a = add(&e.db, parcela(1));
    e.stav(0, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 1, &[]);
    e.run(now()).unwrap();
    let before = e.item(a);

    // budget = 500 - 489 - 6 = 5; after StavUctu 4 left -> the item is admitted.
    // GET 503,503,404 uses 3 attempts; the re-resolve would need 3 more but only 1 is left:
    // the item is deferred instead of overshooting (unguarded it would reach 499).
    let later = now() + Dur::days(15);
    e.stav(489, 500);
    push_503s(&e.mock, "/Parcely/");
    e.mock.push("/Parcely/", 404, "");
    let s = e.run(later).unwrap();
    assert_eq!(s.status, RunStatus::Partial);
    assert_eq!((s.checked, s.errors, s.deferred), (0, 0, 1));
    assert_eq!(s.api_calls, 4);
    assert!(s.api_used_today <= s.api_limit - 6, "{}", s.api_used_today);
    assert_eq!(e.statuses(s.run_id), vec![(a, "deferred".to_string())]);
    let after = e.item(a);
    assert_eq!(after.last_data, before.last_data);
    assert_eq!(after.last_success_at, before.last_success_at);
    assert!(e.db.get_resolved(a).unwrap().is_none());
    assert!(e
        .db
        .due_items(later, 14)
        .unwrap()
        .iter()
        .any(|r| r.item.id == a));

    // Every call needs two retries: budget 14 fits exactly two new items (6 attempts each).
    let b = add(&e.db, parcela(2));
    let c = add(&e.db, parcela(3));
    e.stav(480, 500);
    for k in [102, 103] {
        push_503s(&e.mock, "/Parcely/Vyhledani");
        vyhledani(&e.mock, k);
        push_503s(&e.mock, "/Parcely/");
        get_parcela(&e.mock, k, 1, &[]);
    }
    let s = e.run(later + Dur::hours(1)).unwrap();
    assert_eq!(
        e.statuses(s.run_id),
        vec![
            (b, "first".to_string()),
            (c, "first".to_string()),
            (a, "deferred".to_string())
        ]
    );
    assert_eq!(s.api_calls, 13);
    assert!(s.api_used_today <= s.api_limit - 6, "{}", s.api_used_today);
}

#[test]
fn ciselniky_wait_when_budget_cannot_cover_them() {
    let mut e = setup();
    let j = add(
        &e.db,
        ItemIdent::Jednotka {
            ku: 610704,
            typ: TypStavby::Cp,
            cislo_domovni: 893,
            cislo_jednotky: 37,
            kod_casti_obce: None,
        },
    );
    let p = add(&e.db, parcela(1));
    // budget 6, 5 left after StavUctu: < 9 needed for the three lists with retries.
    e.stav(488, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 1, &[]);
    let s = e.run(now()).unwrap();
    assert_eq!(s.status, RunStatus::Partial);
    assert_eq!(
        e.statuses(s.run_id),
        vec![(j, "deferred".to_string()), (p, "first".to_string())]
    );
    assert_eq!(s.api_calls, 3);
    assert!(e
        .mock
        .requested_urls()
        .iter()
        .all(|u| !u.contains("Ciselniky")));
}

#[test]
fn resolve_error_is_stored_on_item_and_cleared_on_success() {
    let mut e = setup();
    let id = add(&e.db, parcela(182));
    e.stav(10, 500);
    e.mock.push("/Parcely/Vyhledani", 200, r#"{"data":[]}"#);
    let s = e.run(now()).unwrap();
    assert_eq!(s.errors, 1);
    assert_eq!(e.statuses(s.run_id), vec![(id, "error".to_string())]);
    assert!(e.item(id).resolve_error.is_some());

    e.stav(12, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 1, &[]);
    let s = e.run(now() + Dur::hours(1)).unwrap();
    assert_eq!(e.statuses(s.run_id), vec![(id, "first".to_string())]);
    assert!(e.item(id).resolve_error.is_none());
}

#[test]
fn jednotka_without_override_fetches_ciselniky_first() {
    let mut e = setup();
    let id = add(
        &e.db,
        ItemIdent::Jednotka {
            ku: 610704,
            typ: TypStavby::Cp,
            cislo_domovni: 893,
            cislo_jednotky: 37,
            kod_casti_obce: None,
        },
    );
    e.stav(10, 500);
    e.mock.push(
        "/CiselnikyUzemnichJednotek/KatastralniUzemi",
        200,
        r#"{"data":[{"kod":610704,"nazev":"Ponava","kodObce":582786}]}"#,
    );
    e.mock.push(
        "/CiselnikyUzemnichJednotek/Obce",
        200,
        r#"{"data":[{"kod":582786,"nazev":"Brno"}]}"#,
    );
    e.mock.push(
        "/CiselnikyUzemnichJednotek/CastiObci",
        200,
        r#"{"data":[{"kod":411973,"nazev":"Ponava","kodObce":582786}]}"#,
    );
    e.mock
        .push("/Jednotky/Vyhledani", 200, r#"{"data":[{"id":4242}]}"#);
    e.mock.push(
        "/Jednotky/",
        200,
        r#"{"data":{"id":4242,"rizeniPlomby":null}}"#,
    );
    let s = e.run(now()).unwrap();
    assert_eq!(s.api_calls, 6);
    assert_eq!(e.statuses(s.run_id), vec![(id, "first".to_string())]);

    // Resolution cached -> no číselník refresh even when they are stale.
    let later = now() + Dur::days(45);
    e.stav(0, 500);
    e.mock.push(
        "/Jednotky/",
        200,
        r#"{"data":{"id":4242,"rizeniPlomby":[]}}"#,
    );
    let s = e.run(later).unwrap();
    assert_eq!(s.api_calls, 2);
    assert_eq!(e.statuses(s.run_id), vec![(id, "unchanged".to_string())]);
}

#[test]
fn ciselniky_failure_errors_only_items_needing_them() {
    let mut e = setup();
    let j = add(
        &e.db,
        ItemIdent::Jednotka {
            ku: 610704,
            typ: TypStavby::Cp,
            cislo_domovni: 893,
            cislo_jednotky: 37,
            kod_casti_obce: None,
        },
    );
    let p = add(&e.db, parcela(1));
    e.stav(10, 500);
    e.mock
        .push("/CiselnikyUzemnichJednotek/KatastralniUzemi", 400, "bad");
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 1, &[]);
    let s = e.run(now()).unwrap();
    assert_eq!((s.checked, s.errors), (1, 1));
    assert_eq!(
        e.statuses(s.run_id),
        vec![(j, "error".to_string()), (p, "first".to_string())]
    );
}

#[test]
fn invalid_key_aborts_run_without_touching_items() {
    let mut e = setup();
    let a = add(&e.db, parcela(1));
    let b = add(&e.db, parcela(2));
    e.stav(10, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 1, &[]);
    vyhledani(&e.mock, 102);
    get_parcela(&e.mock, 102, 2, &[]);
    e.run(now()).unwrap();
    let results_before = e.db.count_item_results().unwrap();
    let (ra, rb) = (e.item(a), e.item(b));

    // Mid-run: first item fetched fine (changed!), second returns 401 -> whole run aborts.
    let later = now() + Dur::days(15);
    e.stav(0, 500);
    get_parcela(&e.mock, 101, 999, &[]);
    e.mock.push("/Parcely/", 401, "Unauthorized");
    assert_eq!(e.run(later).err(), Some(CoreError::InvalidApiKey));
    let run = &e.db.recent_runs(1).unwrap()[0];
    assert_eq!(run.status, "failed");
    assert_eq!(run.error.as_deref(), Some("API klíč je neplatný"));
    assert!(run.finished_at.is_some());
    assert_eq!(run.api_calls, 3);
    assert_eq!(e.db.count_item_results().unwrap(), results_before);
    assert_eq!(e.item(a).last_data, ra.last_data);
    assert_eq!(e.item(a).last_success_at, ra.last_success_at);
    assert_eq!(e.item(b).last_data, rb.last_data);
    assert!(e.db.unseen_findings().unwrap().is_empty());

    // From StavUctu itself.
    e.mock.push("/AplikacniSluzby/StavUctu", 403, "Forbidden");
    assert_eq!(e.run(later).err(), Some(CoreError::InvalidApiKey));
    let run = &e.db.recent_runs(1).unwrap()[0];
    assert_eq!(run.status, "failed");
    assert_eq!(run.api_calls, 1);
    assert_eq!(e.db.count_item_results().unwrap(), results_before);

    // Other StavUctu failures: failed run, error returned.
    e.mock.push("/AplikacniSluzby/StavUctu", 400, "bad");
    assert!(matches!(
        e.run(later),
        Err(CoreError::Http { status: 400, .. })
    ));
    let run = &e.db.recent_runs(1).unwrap()[0];
    assert_eq!(run.status, "failed");
    assert_eq!(run.error.as_deref(), Some("HTTP chyba 400"));
    assert_eq!(e.item(a).last_success_at, ra.last_success_at);
}

/// Runs a closure once, just before the first request whose URL contains `trigger`, to
/// simulate the UI editing the database while a check is running.
struct HookTransport {
    inner: Arc<MockTransport>,
    hook: Mutex<Option<(String, Hook)>>,
}

type Hook = Box<dyn FnOnce() + Send>;

impl Transport for HookTransport {
    fn get(&self, url: &str, api_key: &str) -> Result<(u16, String), CoreError> {
        let fire = {
            let mut h = self.hook.lock().unwrap();
            if h.as_ref().is_some_and(|(t, _)| url.contains(t.as_str())) {
                h.take().map(|(_, f)| f)
            } else {
                None
            }
        };
        if let Some(f) = fire {
            f();
        }
        self.inner.get(url, api_key)
    }
}

struct FileEnv {
    mock: Arc<MockTransport>,
    hook: Arc<HookTransport>,
    client: ApiClient,
    db: Db,
    db_path: PathBuf,
    lock: PathBuf,
    _dir: TempDir,
}

fn file_setup() -> FileEnv {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("strazce.db");
    let mock = Arc::new(MockTransport::new());
    let hook = Arc::new(HookTransport {
        inner: mock.clone(),
        hook: Mutex::new(None),
    });
    let client = ApiClient::new(hook.clone(), "fake-key".into()).with_retry_delay(Duration::ZERO);
    FileEnv {
        mock,
        hook,
        client,
        db: Db::open(&db_path).unwrap(),
        lock: dir.path().join("strazce.lock"),
        db_path,
        _dir: dir,
    }
}

impl FileEnv {
    fn stav(&self) {
        self.mock.push(
            "/AplikacniSluzby/StavUctu",
            200,
            r#"{"idSluzby":1,"aktualniObdobi":"2026-11-17","provedenoVolani":0,"limitVolani":500,"expiraceApiKey":null}"#,
        );
    }

    /// Runs `f` on a second connection (like the UI) right before a request to `trigger`.
    fn on_request(&self, trigger: &str, f: impl FnOnce(&Db) + Send + 'static) {
        let path = self.db_path.clone();
        *self.hook.hook.lock().unwrap() = Some((
            trigger.to_string(),
            Box::new(move || f(&Db::open(&path).unwrap())),
        ));
    }

    fn run(&mut self, now: DateTime<Utc>) -> Result<strazce_core::run::RunSummary, CoreError> {
        run_check(&self.client, &mut self.db, &self.lock, Trigger::Manual, now)
    }

    fn item(&self, id: i64) -> Option<ItemRow> {
        self.db
            .list_items()
            .unwrap()
            .into_iter()
            .find(|r| r.item.id == id)
    }
}

#[test]
fn identity_change_during_run_is_not_overwritten() {
    let mut e = file_setup();
    let a = add(&e.db, parcela(1));
    e.stav();
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 500, &[]);
    e.run(now()).unwrap();

    // While the old object is being fetched, the user changes the item to another parcel.
    let later = now() + Dur::days(15);
    e.stav();
    e.on_request("/Parcely/101", move |db| {
        db.update_item(&Item {
            id: a,
            popis: "jiná".into(),
            enabled: true,
            ident: parcela(99),
        })
        .unwrap();
        db.set_resolve_error(a, Some("chyba z UI".into())).unwrap();
    });
    get_parcela(&e.mock, 101, 450, &[]);
    let s = e.run(later).unwrap();

    assert_eq!(s.status, RunStatus::Ok);
    assert!(e.db.item_results_for_run(s.run_id).unwrap().is_empty());
    let row = e.item(a).unwrap();
    assert_eq!(
        row.last_data, None,
        "old object's snapshot written onto the new identity"
    );
    assert_eq!(row.last_success_at, None);
    assert_eq!(row.resolve_error.as_deref(), Some("chyba z UI"));
    assert!(e.db.unseen_findings().unwrap().is_empty());
}

#[test]
fn item_deleted_during_run_does_not_fail_the_run() {
    let mut e = file_setup();
    let a = add(&e.db, parcela(1));
    let b = add(&e.db, parcela(2));
    e.stav();
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 500, &[]);
    vyhledani(&e.mock, 102);
    get_parcela(&e.mock, 102, 300, &[]);
    e.run(now()).unwrap();

    let later = now() + Dur::days(15);
    e.stav();
    e.on_request("/Parcely/101", move |db| db.delete_item(a).unwrap());
    get_parcela(&e.mock, 101, 450, &[]);
    get_parcela(&e.mock, 102, 300, &[]);
    let s = e
        .run(later)
        .expect("a deleted item must not fail the whole run");

    assert_eq!(s.status, RunStatus::Ok);
    assert_eq!((s.checked, s.changed), (1, 0));
    let res = e.db.item_results_for_run(s.run_id).unwrap();
    assert_eq!(
        res.iter()
            .map(|r| (r.item_id, r.status.as_str()))
            .collect::<Vec<_>>(),
        vec![(b, "unchanged")]
    );
    assert!(e.item(a).is_none());
    assert_eq!(e.item(b).unwrap().last_success_at, Some(later));
    let run = &e.db.recent_runs(1).unwrap()[0];
    assert_eq!((run.status.as_str(), run.checked), ("ok", 1));
}

#[test]
fn plomba_present_at_first_check_is_reported() {
    let mut e = setup();
    let a = add(&e.db, parcela(1));
    let b = add(&e.db, parcela(2));
    e.stav(10, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 500, &[7]);
    e.mock.push(
        "/Rizeni/",
        200,
        r#"{"data":{"id":7,"datumPrijeti":"2026-11-01T09:00:00","stav":"probíhá","provedeneOperace":[]}}"#,
    );
    vyhledani(&e.mock, 102);
    get_parcela(&e.mock, 102, 300, &[]);

    let s = e.run(now()).unwrap();
    assert_eq!(s.status, RunStatus::Ok);
    assert_eq!(
        (s.checked, s.changed, s.plomby_nove, s.errors),
        (2, 0, 1, 0)
    );
    assert_eq!(s.api_calls, 6); // StavUctu + 2×(Vyhledani + GET) + Rizeni detail
    assert_eq!(
        e.statuses(s.run_id),
        vec![(a, "first".to_string()), (b, "first".to_string())]
    );

    let findings = e.db.unseen_findings().unwrap();
    assert_eq!(
        findings.len(),
        1,
        "only the first check with a plomba is a finding"
    );
    assert_eq!(
        (findings[0].item_id, findings[0].status.as_str()),
        (a, "first")
    );
    let c: Changes = serde_json::from_str(findings[0].changes_json.as_deref().unwrap()).unwrap();
    assert!(c.fields.is_empty() && c.plomby_zanikle.is_empty());
    assert_eq!(c.plomby_nove.len(), 1);
    assert_eq!(c.plomby_nove[0].rizeni.id, 7);
    assert_eq!(c.plomby_nove[0].stav.as_deref(), Some("probíhá"));
    let res = e.db.item_results_for_run(s.run_id).unwrap();
    assert!(res[1].changes_json.is_none());

    // The plomba is part of the stored state: next time it is not new again.
    let later = now() + Dur::days(15);
    e.stav(0, 500);
    get_parcela(&e.mock, 101, 500, &[7]);
    get_parcela(&e.mock, 102, 300, &[]);
    let s = e.run(later).unwrap();
    assert_eq!((s.changed, s.plomby_nove), (0, 0));
}

#[test]
fn stale_running_row_of_a_killed_process_is_marked_failed() {
    let mut e = setup();
    // A previous process died mid-run and left its row in `running`.
    let stale = e.db.begin_run("scheduled", now() - Dur::hours(3)).unwrap();
    let s = e.run(now()).unwrap();
    let runs = e.db.recent_runs(5).unwrap();
    let old = runs.iter().find(|r| r.id == stale).unwrap();
    assert_eq!(old.status, "failed");
    assert_eq!(old.error.as_deref(), Some("Kontrola byla přerušena."));
    let new = runs.iter().find(|r| r.id == s.run_id).unwrap();
    assert_eq!(new.status, "ok");

    // A run that cannot take the lock leaves the other process's row alone.
    let held = RunLock::acquire(&e.lock).unwrap();
    let running = e.db.begin_run("manual", now()).unwrap();
    assert_eq!(e.run(now()).err(), Some(CoreError::AlreadyRunning));
    let r = e.db.recent_runs(1).unwrap();
    assert_eq!((r[0].id, r[0].status.as_str()), (running, "running"));
    drop(held);
}

#[test]
fn unparseable_old_plomby_do_not_produce_false_new_plomby() {
    let mut e = setup();
    let id = add(&e.db, parcela(1));
    e.stav(10, 500);
    vyhledani(&e.mock, 101);
    get_parcela(&e.mock, 101, 500, &[]);
    e.run(now()).unwrap();
    // A stored snapshot whose plomby cannot be read (e.g. an older format).
    e.db.set_item_success(id, r#"{"id":101,"vymera":500,"rizeniPlomby":"???"}"#, now())
        .unwrap();

    let later = now() + Dur::days(15);
    e.stav(0, 500);
    get_parcela(&e.mock, 101, 500, &[5]);
    e.mock.push("/Rizeni/", 400, "bad"); // only consumed by a (wrong) detail lookup
    let s = e.run(later).unwrap();
    assert_eq!((s.changed, s.plomby_nove, s.errors), (0, 0, 0));
    assert_eq!(e.statuses(s.run_id), vec![(id, "unchanged".to_string())]);
    // The new, readable snapshot is the baseline from now on.
    let data: serde_json::Value =
        serde_json::from_str(e.item(id).last_data.as_deref().unwrap()).unwrap();
    assert_eq!(data["rizeniPlomby"][0]["id"], 5);
}

#[test]
fn plomba_with_missing_fields_is_still_reported() {
    let mut e = setup();
    let id = add(&e.db, parcela(1));
    e.stav(10, 500);
    vyhledani(&e.mock, 101);
    e.mock.push(
        "/Parcely/",
        200,
        r#"{"data":{"id":101,"vymera":1,"rizeniPlomby":[{"id":8,"typRizeni":"V"}]}}"#,
    );
    e.mock.push("/Rizeni/", 400, "bad");
    let s = e.run(now()).unwrap();
    assert_eq!((s.errors, s.plomby_nove), (0, 1));
    assert_eq!(e.statuses(s.run_id), vec![(id, "first".to_string())]);
    let f = &e.db.unseen_findings().unwrap()[0];
    let c: Changes = serde_json::from_str(f.changes_json.as_deref().unwrap()).unwrap();
    assert_eq!(c.plomby_nove[0].rizeni.label(), "V-?/?-?");
}
