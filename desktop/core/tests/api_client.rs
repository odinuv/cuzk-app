use std::sync::Arc;
use std::time::Duration;
use strazce_core::api::{ApiClient, Envelope, MockTransport, RizeniDef};
use strazce_core::error::CoreError;

fn client(mock: &Arc<MockTransport>) -> ApiClient {
    ApiClient::new(mock.clone(), "fake-key".into()).with_retry_delay(Duration::ZERO)
}

#[test]
fn envelope_parsed() {
    let mock = Arc::new(MockTransport::new());
    mock.push("/Parcely/1", 200, r#"{"data":{"id":5},"aktualnostDatK":"2026-01-01","provedenoVolani":7}"#);
    let env: Envelope<serde_json::Value> = client(&mock).get("/Parcely/1", &[]).unwrap();
    assert_eq!(env.data["id"], 5);
    assert_eq!(env.provedeno_volani, Some(7));
    assert_eq!(env.aktualnost_dat_k.as_deref(), Some("2026-01-01"));
}

#[test]
fn stav_uctu_flat() {
    let mock = Arc::new(MockTransport::new());
    mock.push("/AplikacniSluzby/StavUctu", 200,
        r#"{"idSluzby":1,"aktualniObdobi":"x","provedenoVolani":12,"limitVolani":500,"expiraceApiKey":"2027-01-01"}"#);
    let s = client(&mock).stav_uctu().unwrap();
    assert_eq!(s.limit_volani, 500);
    assert_eq!(s.provedeno_volani, 12);
}

#[test]
fn retries_then_succeeds() {
    let mock = Arc::new(MockTransport::new());
    mock.push("/X", 503, "");
    mock.push("/X", 503, "");
    mock.push("/X", 200, r#"{"data":1}"#);
    let c = client(&mock);
    assert!(c.get::<i64>("/X", &[]).is_ok());
    assert_eq!(c.calls(), 3);
}

#[test]
fn gives_up_after_two_retries() {
    let mock = Arc::new(MockTransport::new());
    for _ in 0..3 { mock.push("/X", 500, "boom"); }
    let c = client(&mock);
    let err = c.get::<i64>("/X", &[]).unwrap_err();
    assert_eq!(err, CoreError::Http { status: 500, body: "boom".into() });
    assert_eq!(c.calls(), 3);
}

#[test]
fn not_found_no_retry() {
    let mock = Arc::new(MockTransport::new());
    mock.push("/X", 404, "");
    let c = client(&mock);
    assert_eq!(c.get::<i64>("/X", &[]).unwrap_err(), CoreError::NotFound);
    assert_eq!(c.calls(), 1);
}

#[test]
fn unauthorized_maps_invalid_key() {
    let mock = Arc::new(MockTransport::new());
    mock.push("/X", 401, "");
    mock.push("/Y", 403, "");
    let c = client(&mock);
    assert_eq!(c.get::<i64>("/X", &[]).unwrap_err(), CoreError::InvalidApiKey);
    assert_eq!(c.get::<i64>("/Y", &[]).unwrap_err(), CoreError::InvalidApiKey);
}

#[test]
fn sends_api_key_header_and_query() {
    let mock = Arc::new(MockTransport::new());
    mock.push("/Parcely/Vyhledani", 200, r#"{"data":[]}"#);
    client(&mock)
        .get::<Vec<i64>>("/Parcely/Vyhledani", &[("KodKatastralnihoUzemi", "610771".into()), ("Text", "a b&č".into())])
        .unwrap();
    let urls = mock.requested_urls();
    assert!(urls[0].starts_with("https://api-kn.cuzk.gov.cz/api/v1/Parcely/Vyhledani?"));
    assert!(urls[0].contains("KodKatastralnihoUzemi=610771"));
    assert!(urls[0].contains("Text=a%20b%26%C4%8D"));
    assert_eq!(mock.api_keys_seen(), vec!["fake-key".to_string()]);
}

#[test]
fn longest_prefix_wins() {
    let mock = Arc::new(MockTransport::new());
    mock.push("/Parcely", 200, r#"{"data":1}"#);
    mock.push("/Parcely/Vyhledani", 200, r#"{"data":2}"#);
    let c = client(&mock);
    assert_eq!(c.get::<i64>("/Parcely/Vyhledani", &[]).unwrap().data, 2);
    assert_eq!(c.get::<i64>("/Parcely/5", &[]).unwrap().data, 1);
}

#[test]
fn rizeni_label() {
    let r = RizeniDef { id: 1, typ_rizeni: "V".into(), poradove_cislo: 1234, rok: 2026, kod_pracoviste: 701 };
    assert_eq!(r.label(), "V-1234/2026-701");
}
