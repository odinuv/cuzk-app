use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope<T> {
    pub data: T,
    #[serde(default)]
    pub aktualnost_dat_k: Option<String>,
    #[serde(default)]
    pub provedeno_volani: Option<u32>,
}

/// Flat (non-enveloped) body of `GET /AplikacniSluzby/StavUctu`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StavUctu {
    pub provedeno_volani: u32,
    pub limit_volani: u32,
    #[serde(default)]
    pub expirace_api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RizeniDef {
    pub id: i64,
    pub typ_rizeni: String,
    pub poradove_cislo: u32,
    pub rok: u32,
    pub kod_pracoviste: u32,
}

impl RizeniDef {
    /// e.g. `V-1234/2026-701`
    pub fn label(&self) -> String {
        format!(
            "{}-{}/{}-{}",
            self.typ_rizeni, self.poradove_cislo, self.rok, self.kod_pracoviste
        )
    }
}
