//! Resolution of user-entered identifications to ISKN ids, plus the číselník cache.

use crate::api::ApiClient;
use crate::db::{normalize_cz, Db, ItemRow, Resolved};
use crate::error::CoreError;
use crate::model::{item_key, ItemIdent, ParcelaRef};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use std::str::FromStr;

const CISELNIK_MAX_AGE_DAYS: i64 = 30;
const NOT_FOUND: &str = "Objekt nenalezen v katastru – zkontrolujte zadání.";
const AMBIGUOUS_CAST: &str = "Nepodařilo se jednoznačně určit část obce – vyberte ji ručně.";

/// ISKN endpoint family; the string form is the URL path segment and the value
/// stored in `resolved.endpoint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    Parcely,
    Stavby,
    Jednotky,
}

impl Endpoint {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Parcely => "Parcely",
            Self::Stavby => "Stavby",
            Self::Jednotky => "Jednotky",
        }
    }
}

impl FromStr for Endpoint {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, CoreError> {
        match s {
            "Parcely" => Ok(Self::Parcely),
            "Stavby" => Ok(Self::Stavby),
            "Jednotky" => Ok(Self::Jednotky),
            other => Err(CoreError::Db(format!("Neznámý endpoint: {other}"))),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CiselnikRow {
    kod: u32,
    nazev: String,
    #[serde(default)]
    kod_obce: Option<u32>,
    #[serde(default)]
    platnost_do: Option<String>,
}

fn is_stale(age: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    match age {
        None => true,
        Some(t) => now - t > Duration::days(CISELNIK_MAX_AGE_DAYS),
    }
}

fn fetch_list(client: &ApiClient, name: &str) -> Result<Vec<CiselnikRow>, CoreError> {
    let path = format!("/CiselnikyUzemnichJednotek/{name}");
    Ok(client.get::<Vec<CiselnikRow>>(&path, &[])?.data)
}

/// Refreshes the KatastralniUzemi, Obce and CastiObci lists, each independently,
/// when its stored copy is missing or older than 30 days. Returns the number of
/// API calls made (0 when everything is fresh). Expired (`platnostDo`) části obce
/// are not stored.
pub fn ensure_ciselniky(
    client: &ApiClient,
    db: &Db,
    now: DateTime<Utc>,
) -> Result<u32, CoreError> {
    let mut calls = 0;
    if is_stale(db.ciselnik_age("ku")?, now) {
        let rows = fetch_list(client, "KatastralniUzemi")?;
        calls += 1;
        let rows: Vec<_> = rows
            .into_iter()
            .map(|r| (r.kod, r.nazev, r.kod_obce.unwrap_or(0)))
            .collect();
        db.replace_ku(&rows, now)?;
    }
    if is_stale(db.ciselnik_age("obce")?, now) {
        let rows = fetch_list(client, "Obce")?;
        calls += 1;
        let rows: Vec<_> = rows.into_iter().map(|r| (r.kod, r.nazev)).collect();
        db.replace_obce(&rows, now)?;
    }
    if is_stale(db.ciselnik_age("casti_obci")?, now) {
        let rows = fetch_list(client, "CastiObci")?;
        calls += 1;
        let rows: Vec<_> = rows
            .into_iter()
            .filter(|r| r.platnost_do.is_none())
            .map(|r| (r.kod, r.nazev, r.kod_obce.unwrap_or(0)))
            .collect();
        db.replace_casti_obci(&rows, now)?;
    }
    Ok(calls)
}

#[derive(Deserialize)]
struct StavbaLookup {
    id: i64,
    #[serde(rename = "cislaDomovni", default)]
    cisla_domovni: Option<Vec<u32>>,
}

#[derive(Deserialize)]
struct ParcelaLookup {
    id: i64,
    #[serde(default)]
    stavba: Option<StavbaLookup>,
}

#[derive(Deserialize)]
struct IdOnly {
    id: i64,
}

fn not_found() -> CoreError {
    CoreError::Resolve(NOT_FOUND.into())
}

fn lookup_parcela(
    client: &ApiClient,
    ku: u32,
    p: &ParcelaRef,
) -> Result<ParcelaLookup, CoreError> {
    let mut q: Vec<(&str, String)> = vec![
        ("KodKatastralnihoUzemi", ku.to_string()),
        ("TypParcely", "PKN".into()),
        ("DruhCislovaniParcely", p.druh.api_code().to_string()),
        ("KmenoveCisloParcely", p.kmenove.to_string()),
    ];
    if let Some(d) = p.poddeleni {
        q.push(("PoddeleniCislaParcely", d.to_string()));
    }
    let mut data = client.get::<Vec<ParcelaLookup>>("/Parcely/Vyhledani", &q)?.data;
    match data.len() {
        0 => Err(not_found()),
        1 => Ok(data.remove(0)),
        _ => Err(CoreError::Resolve(
            "Parcelu nelze jednoznačně určit – nalezeno více shod, zkontrolujte zadání.".into(),
        )),
    }
}

fn resolve_cast_obce(db: &Db, ku: u32) -> Result<u32, CoreError> {
    let (nazev, kod_obce) = db.get_ku(ku)?.ok_or_else(|| {
        CoreError::Resolve(
            "Katastrální území není v číselníku – nejprve aktualizujte číselníky.".into(),
        )
    })?;
    let want = normalize_cz(&nazev);
    let matches: Vec<u32> = db
        .casti_obci_for_obec(kod_obce)?
        .into_iter()
        .filter(|(_, n)| normalize_cz(n) == want)
        .map(|(k, _)| k)
        .collect();
    match matches.as_slice() {
        [one] => Ok(*one),
        _ => Err(CoreError::Resolve(AMBIGUOUS_CAST.into())),
    }
}

fn lookup(client: &ApiClient, db: &Db, ident: &ItemIdent) -> Result<(Endpoint, i64), CoreError> {
    match ident {
        ItemIdent::Parcela { ku, parcela } => {
            Ok((Endpoint::Parcely, lookup_parcela(client, *ku, parcela)?.id))
        }
        ItemIdent::Stavba { ku, cislo_domovni, na_parcele, .. } => {
            let stavba = lookup_parcela(client, *ku, na_parcele)?.stavba.ok_or_else(not_found)?;
            if !stavba.cisla_domovni.unwrap_or_default().contains(cislo_domovni) {
                return Err(not_found());
            }
            Ok((Endpoint::Stavby, stavba.id))
        }
        ItemIdent::Jednotka { ku, typ, cislo_domovni, cislo_jednotky, kod_casti_obce } => {
            let cast = match kod_casti_obce {
                Some(k) => *k,
                None => resolve_cast_obce(db, *ku)?,
            };
            let q = [
                ("KodCastiObce", cast.to_string()),
                ("TypStavby", typ.api_code().to_string()),
                ("CisloDomovni", cislo_domovni.to_string()),
                ("CisloJednotky", cislo_jednotky.to_string()),
            ];
            let mut data = client.get::<Vec<IdOnly>>("/Jednotky/Vyhledani", &q)?.data;
            if data.is_empty() {
                return Err(not_found());
            }
            Ok((Endpoint::Jednotky, data.remove(0).id))
        }
    }
}

/// Returns the cached ISKN id when `resolved.item_key` still matches the item's
/// identification, otherwise looks it up (and caches it). Does not refresh
/// číselníky: for a jednotka without `kod_casti_obce` the caller must have run
/// `ensure_ciselniky` first.
pub fn resolve(
    client: &ApiClient,
    db: &Db,
    item: &ItemRow,
) -> Result<(Endpoint, i64), CoreError> {
    let key = item_key(&item.item.ident);
    if let Some(r) = db.get_resolved(item.item.id)? {
        if r.item_key == key {
            return Ok((r.endpoint.parse()?, r.iskn_id));
        }
    }
    let (endpoint, id) = lookup(client, db, &item.item.ident)?;
    db.set_resolved(
        item.item.id,
        &Resolved { item_key: key, endpoint: endpoint.as_str().to_string(), iskn_id: id },
    )?;
    Ok((endpoint, id))
}
