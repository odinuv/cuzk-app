use chrono::{DateTime, Duration as Dur, TimeZone, Utc};
use std::sync::Arc;
use std::time::Duration;
use strazce_core::api::{ApiClient, MockTransport};
use strazce_core::db::{Db, ItemRow, NewItem};
use strazce_core::error::CoreError;
use strazce_core::model::*;
use strazce_core::resolve::{ensure_ciselniky, resolve, Endpoint};

const NOT_FOUND: &str = "Objekt nenalezen v katastru – zkontrolujte zadání.";
const AMBIGUOUS: &str = "Nepodařilo se jednoznačně určit část obce – vyberte ji ručně.";

fn setup() -> (Arc<MockTransport>, ApiClient, Db) {
    let mock = Arc::new(MockTransport::new());
    let client = ApiClient::new(mock.clone(), "fake-key".into()).with_retry_delay(Duration::ZERO);
    (mock, client, Db::open_in_memory().unwrap())
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 17, 8, 0, 0).unwrap()
}

fn add(db: &Db, ident: ItemIdent) -> ItemRow {
    let id = db
        .insert_item(&NewItem {
            popis: "t".into(),
            enabled: true,
            ident,
        })
        .unwrap();
    db.list_items()
        .unwrap()
        .into_iter()
        .find(|r| r.item.id == id)
        .unwrap()
}

fn parcela(p: Option<u32>) -> ItemIdent {
    ItemIdent::Parcela {
        ku: 618853,
        parcela: ParcelaRef {
            druh: DruhCislovani::Stavebni,
            kmenove: 182,
            poddeleni: p,
        },
    }
}

fn jednotka(override_cast: Option<u32>) -> ItemIdent {
    ItemIdent::Jednotka {
        ku: 610704,
        typ: TypStavby::Cp,
        cislo_domovni: 893,
        cislo_jednotky: 37,
        kod_casti_obce: override_cast,
    }
}

fn seed_brno(db: &Db) {
    db.replace_ku(&[(610704, "Ponava".into(), 582786)], now())
        .unwrap();
    db.replace_casti_obci(
        &[
            (411973, "Ponava".into(), 582786),
            (411981, "Zábrdovice".into(), 582786),
            (999, "Ponava".into(), 111),
        ],
        now(),
    )
    .unwrap();
}

#[test]
fn parcela_with_and_without_poddeleni() {
    let (mock, client, db) = setup();
    mock.push(
        "/Parcely/Vyhledani",
        200,
        r#"{"data":[{"id":101,"stavba":null}]}"#,
    );
    mock.push("/Parcely/Vyhledani", 200, r#"{"data":[{"id":102}]}"#);

    let r = add(&db, parcela(None));
    assert_eq!(resolve(&client, &db, &r).unwrap(), (Endpoint::Parcely, 101));
    let r2 = add(&db, parcela(Some(2)));
    assert_eq!(
        resolve(&client, &db, &r2).unwrap(),
        (Endpoint::Parcely, 102)
    );

    let urls = mock.requested_urls();
    assert!(urls[0].contains("KodKatastralnihoUzemi=618853"));
    assert!(urls[0].contains("TypParcely=PKN"));
    assert!(urls[0].contains("DruhCislovaniParcely=1"));
    assert!(urls[0].contains("KmenoveCisloParcely=182"));
    assert!(!urls[0].contains("PoddeleniCislaParcely"));
    assert!(urls[1].contains("PoddeleniCislaParcely=2"));

    // zero and multiple matches
    mock.push("/Parcely/Vyhledani", 200, r#"{"data":[]}"#);
    let r3 = add(&db, parcela(Some(3)));
    match resolve(&client, &db, &r3) {
        Err(CoreError::Resolve(m)) => assert_eq!(m, NOT_FOUND),
        other => panic!("{other:?}"),
    }
    mock.push("/Parcely/Vyhledani", 200, r#"{"data":[{"id":1},{"id":2}]}"#);
    let r4 = add(&db, parcela(Some(4)));
    assert!(matches!(
        resolve(&client, &db, &r4),
        Err(CoreError::Resolve(_))
    ));
}

fn stavba(cd: u32) -> ItemIdent {
    ItemIdent::Stavba {
        ku: 738395,
        typ: TypStavby::Cp,
        cislo_domovni: cd,
        na_parcele: ParcelaRef {
            druh: DruhCislovani::Stavebni,
            kmenove: 10,
            poddeleni: None,
        },
    }
}

#[test]
fn stavba_via_parcel_checks_cislo_domovni() {
    let (mock, client, db) = setup();
    let body = r#"{"data":[{"id":5,"stavba":{"id":777,"cislaDomovni":[12,14]}}]}"#;
    mock.push("/Parcely/Vyhledani", 200, body);
    let ok = add(&db, stavba(14));
    assert_eq!(resolve(&client, &db, &ok).unwrap(), (Endpoint::Stavby, 777));

    mock.push("/Parcely/Vyhledani", 200, body);
    let bad = add(&db, stavba(99));
    match resolve(&client, &db, &bad) {
        Err(CoreError::Resolve(m)) => assert_eq!(m, NOT_FOUND),
        other => panic!("{other:?}"),
    }
    assert!(db.get_resolved(bad.item.id).unwrap().is_none());

    mock.push(
        "/Parcely/Vyhledani",
        200,
        r#"{"data":[{"id":5,"stavba":null}]}"#,
    );
    let none = add(&db, stavba(15));
    assert!(matches!(
        resolve(&client, &db, &none),
        Err(CoreError::Resolve(_))
    ));
}

#[test]
fn jednotka_picks_cast_obce_by_name() {
    let (mock, client, db) = setup();
    seed_brno(&db);
    mock.push("/Jednotky/Vyhledani", 200, r#"{"data":[{"id":4242}]}"#);
    let r = add(&db, jednotka(None));
    assert_eq!(
        resolve(&client, &db, &r).unwrap(),
        (Endpoint::Jednotky, 4242)
    );
    let u = &mock.requested_urls()[0];
    assert!(u.contains("KodCastiObce=411973"), "{u}");
    assert!(u.contains("TypStavby=1"));
    assert!(u.contains("CisloDomovni=893"));
    assert!(u.contains("CisloJednotky=37"));
}

#[test]
fn jednotka_ambiguous_cast_obce_errors() {
    let (mock, client, db) = setup();
    // zero candidates with that name
    db.replace_ku(&[(610704, "Ponava".into(), 582786)], now())
        .unwrap();
    db.replace_casti_obci(&[(411981, "Zábrdovice".into(), 582786)], now())
        .unwrap();
    let r = add(&db, jednotka(None));
    match resolve(&client, &db, &r) {
        Err(CoreError::Resolve(m)) => assert_eq!(m, AMBIGUOUS),
        other => panic!("{other:?}"),
    }
    // two candidates (differing only in case)
    db.replace_casti_obci(
        &[(1, "Ponava".into(), 582786), (2, "PONAVA".into(), 582786)],
        now(),
    )
    .unwrap();
    match resolve(&client, &db, &r) {
        Err(CoreError::Resolve(m)) => assert_eq!(m, AMBIGUOUS),
        other => panic!("{other:?}"),
    }
    // ku missing from cache
    let r2 = add(
        &db,
        ItemIdent::Jednotka {
            ku: 1,
            typ: TypStavby::Cp,
            cislo_domovni: 1,
            cislo_jednotky: 1,
            kod_casti_obce: None,
        },
    );
    assert!(matches!(
        resolve(&client, &db, &r2),
        Err(CoreError::Resolve(_))
    ));
    assert!(mock.requested_urls().is_empty());
}

#[test]
fn jednotka_override_skips_ciselnik() {
    let (mock, client, db) = setup();
    mock.push("/Jednotky/Vyhledani", 200, r#"{"data":[{"id":9}]}"#);
    let r = add(&db, jednotka(Some(555)));
    assert_eq!(resolve(&client, &db, &r).unwrap(), (Endpoint::Jednotky, 9));
    let urls = mock.requested_urls();
    assert_eq!(urls.len(), 1);
    assert!(urls[0].contains("KodCastiObce=555"));
}

#[test]
fn cache_hit_makes_no_calls() {
    let (mock, client, db) = setup();
    mock.push("/Parcely/Vyhledani", 200, r#"{"data":[{"id":101}]}"#);
    let r = add(&db, parcela(None));
    resolve(&client, &db, &r).unwrap();
    assert_eq!(client.calls(), 1);
    assert_eq!(resolve(&client, &db, &r).unwrap(), (Endpoint::Parcely, 101));
    assert_eq!(client.calls(), 1);
    assert_eq!("Parcely".parse::<Endpoint>().unwrap(), Endpoint::Parcely);
    assert_eq!(Endpoint::Jednotky.as_str(), "Jednotky");
}

fn push_ciselniky(mock: &MockTransport) {
    mock.push(
        "/CiselnikyUzemnichJednotek/KatastralniUzemi",
        200,
        r#"{"data":[{"kod":610704,"nazev":"Ponava","kodObce":582786,"platnostDo":null}]}"#,
    );
    mock.push(
        "/CiselnikyUzemnichJednotek/Obce",
        200,
        r#"{"data":[{"kod":582786,"nazev":"Brno","kodOkresu":3702,"platnostDo":null}]}"#,
    );
    mock.push(
        "/CiselnikyUzemnichJednotek/CastiObci",
        200,
        r#"{"data":[{"kod":411973,"nazev":"Ponava","kodObce":582786,"platnostDo":null},{"kod":5,"nazev":"Stará","kodObce":582786,"platnostDo":"2001-01-01"}]}"#,
    );
}

#[test]
fn ciselniky_refetched_only_when_stale() {
    let (mock, client, db) = setup();
    push_ciselniky(&mock);
    assert_eq!(ensure_ciselniky(&client, &db, now()).unwrap(), 3);
    assert_eq!(client.calls(), 3);
    assert_eq!(db.get_ku(610704).unwrap(), Some(("Ponava".into(), 582786)));
    // expired část obce is dropped
    assert_eq!(
        db.casti_obci_for_obec(582786).unwrap(),
        vec![(411973, "Ponava".to_string())]
    );

    // fresh -> nothing
    assert_eq!(
        ensure_ciselniky(&client, &db, now() + Dur::days(29)).unwrap(),
        0
    );
    assert_eq!(client.calls(), 3);

    // stale -> all three again
    push_ciselniky(&mock);
    assert_eq!(
        ensure_ciselniky(&client, &db, now() + Dur::days(31)).unwrap(),
        3
    );

    // each list independently: only casti_obci stale
    db.replace_ku(&[(610704, "Ponava".into(), 582786)], now() + Dur::days(60))
        .unwrap();
    db.replace_obce(&[(582786, "Brno".into())], now() + Dur::days(60))
        .unwrap();
    mock.push(
        "/CiselnikyUzemnichJednotek/CastiObci",
        200,
        r#"{"data":[]}"#,
    );
    assert_eq!(
        ensure_ciselniky(&client, &db, now() + Dur::days(62)).unwrap(),
        1
    );
}
